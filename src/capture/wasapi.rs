//! A fonte real: o WASAPI pelo cpal. O microfone é um stream de entrada; o som do PC é um
//! stream de entrada aberto no dispositivo de saída (o cpal liga o loopback sozinho), com um
//! stream de silêncio tocando na mesma saída, porque o loopback não entrega pacotes enquanto
//! nada toca (spike C, técnica do OBS). Os callbacks só copiam para os rings.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, InterfaceType, SampleFormat, Stream, StreamConfig};

use super::source::{
    AudioSource, Device, Devices, Error, EventKind, Events, Format, Opened, Producer, RING_SECS,
    Track, ring,
};

pub struct CpalSource {
    host: cpal::Host,
    qpc_frequency: u64,
}

impl Default for CpalSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CpalSource {
    pub fn new() -> Self {
        let mut frequency = 0i64;
        // SAFETY: só escreve no inteiro recebido; nunca falha do Windows XP em diante.
        let _ = unsafe {
            windows::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
        };
        Self {
            host: cpal::default_host(),
            qpc_frequency: frequency.max(1) as u64,
        }
    }

    fn find(&self, track: Track, id: Option<&str>) -> Result<cpal::Device, Error> {
        let missing = match track {
            Track::Mic => Error::NoMic,
            Track::System => Error::NoOutput,
        };
        let default = || match track {
            Track::Mic => self.host.default_input_device(),
            Track::System => self.host.default_output_device(),
        };
        let Some(id) = id else {
            return default().ok_or(missing);
        };
        // O escolhido sumiu: o padrão do Windows.
        let chosen = id.parse().ok().and_then(|id| self.host.device_by_id(&id));
        if chosen.is_none() {
            eprintln!("gravação: o dispositivo {id} não existe mais; usando o padrão");
        }
        chosen.or_else(default).ok_or(missing)
    }
}

/// Bluetooth pelo cpal (enumerador `BTHENUM`) ou pelo enumerador do Windows (`BTHHFENUM` no
/// perfil de chamada, `BTHLE…` no LE Audio), que o cpal não mapeia.
pub fn is_bluetooth(interface: InterfaceType, enumerator: Option<&str>) -> bool {
    interface == InterfaceType::Bluetooth
        || enumerator.is_some_and(|e| e.to_ascii_uppercase().starts_with("BTH"))
}

fn describe(device: &cpal::Device, default_id: Option<&str>) -> Option<Device> {
    let id = device.id().ok()?.to_string();
    let description = device.description().ok()?;
    Some(Device {
        default: default_id == Some(id.as_str()),
        bluetooth: is_bluetooth(description.interface_type(), enumerator(device).as_deref()),
        name: description.name().to_string(),
        id,
    })
}

/// `DEVPKEY_Device_EnumeratorName` do endpoint.
fn enumerator(device: &cpal::Device) -> Option<String> {
    use windows::Win32::Devices::Properties::DEVPKEY_Device_EnumeratorName;
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::System::Variant::VT_LPWSTR;

    let cpal::platform::DeviceInner::Wasapi(inner) = device.as_inner();
    let endpoint = inner.immdevice()?;
    // SAFETY: COM já foi iniciado nesta thread pelo cpal (a enumeração); o valor lido só é
    // usado se for uma string terminada em 0.
    unsafe {
        let store = endpoint.OpenPropertyStore(STGM_READ).ok()?;
        let key = &DEVPKEY_Device_EnumeratorName as *const _ as *const PROPERTYKEY;
        let mut value = store.GetValue(key).ok()?;
        let raw = &value.Anonymous.Anonymous;
        let text = (raw.vt == VT_LPWSTR && !raw.Anonymous.pwszVal.is_null())
            .then(|| raw.Anonymous.pwszVal.to_string().ok())
            .flatten();
        let _ = windows::Win32::System::Com::StructuredStorage::PropVariantClear(&mut value);
        text
    }
}

impl AudioSource for CpalSource {
    fn devices(&self) -> Result<Devices, Error> {
        let default_id =
            |d: Option<cpal::Device>| d.and_then(|d| d.id().ok()).map(|id| id.to_string());
        let default_in = default_id(self.host.default_input_device());
        let default_out = default_id(self.host.default_output_device());
        let list = |devices: Result<_, cpal::Error>, default: Option<&str>| -> Vec<Device> {
            devices
                .map(|ds: Vec<cpal::Device>| {
                    ds.iter().filter_map(|d| describe(d, default)).collect()
                })
                .unwrap_or_else(|e| {
                    eprintln!("gravação: não listei os dispositivos: {e}");
                    Vec::new()
                })
        };
        Ok(Devices {
            inputs: list(
                self.host.input_devices().map(Iterator::collect),
                default_in.as_deref(),
            ),
            outputs: list(
                self.host.output_devices().map(Iterator::collect),
                default_out.as_deref(),
            ),
        })
    }

    fn now_ns(&self) -> u64 {
        let mut counter = 0i64;
        // SAFETY: só escreve no inteiro recebido.
        let _ =
            unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut counter) };
        // A mesma conta do cpal: unidades de 100 ns primeiro, como o QPC dos pacotes.
        let units = counter.max(0) as u128 * 10_000_000 / u128::from(self.qpc_frequency);
        (units * 100) as u64
    }

    fn open(&self, track: Track, device: Option<&str>, events: Events) -> Result<Opened, Error> {
        let device = self.find(track, device)?;
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_default();
        let config = match track {
            Track::Mic => device.default_input_config(),
            Track::System => device.default_output_config(),
        }
        .map_err(|e| open_error(track, &e))?;
        let format = Format {
            rate: config.sample_rate(),
            channels: config.channels(),
        };
        let sample_format = config.sample_format();
        let config = config.config();
        let (producer, consumer) = ring(format, RING_SECS);
        let capture = input(&device, &config, sample_format, producer, events.clone())
            .map_err(|e| open_error(track, &e))?;
        let streams: Box<dyn Send> = match track {
            Track::Mic => {
                capture.play().map_err(|e| open_error(track, &e))?;
                Box::new(capture)
            }
            Track::System => {
                // O silêncio começa antes do loopback: o primeiro pacote já chega.
                let silence = silence(&device, &config, sample_format, events)
                    .map_err(|e| open_error(track, &e))?;
                silence.play().map_err(|e| open_error(track, &e))?;
                capture.play().map_err(|e| open_error(track, &e))?;
                Box::new((silence, capture))
            }
        };
        Ok(Opened {
            consumer,
            name,
            streams,
        })
    }
}

fn open_error(track: Track, e: &cpal::Error) -> Error {
    eprintln!("gravação: {track:?} não abriu: {e}");
    let denied = e.kind() == ErrorKind::PermissionDenied
        || e.message()
            .is_some_and(|m| m.contains("os error 5") || m.contains("0x80070005"));
    match e.kind() {
        _ if denied && track == Track::Mic => Error::MicDenied,
        ErrorKind::DeviceNotAvailable => match track {
            Track::Mic => Error::NoMic,
            Track::System => Error::NoOutput,
        },
        _ => Error::Device(track),
    }
}

fn on_error(events: Events) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e| {
        let kind = match e.kind() {
            ErrorKind::StreamInvalidated | ErrorKind::DeviceChanged => EventKind::Invalidated,
            ErrorKind::DeviceNotAvailable => EventKind::Lost,
            ErrorKind::Xrun => EventKind::Xrun,
            _ => {
                eprintln!("gravação: erro no stream de {:?}: {e}", events.track);
                return;
            }
        };
        events.send(kind);
    }
}

/// O stream de captura (o microfone, ou o loopback de uma saída), convertendo para f32 no
/// caminho para o ring.
fn input(
    device: &cpal::Device,
    config: &StreamConfig,
    format: SampleFormat,
    mut producer: Producer,
    events: Events,
) -> Result<Stream, cpal::Error> {
    let channels = config.channels.max(1);
    macro_rules! build {
        ($t:ty, $to_f32:expr) => {
            device.build_input_stream(
                *config,
                move |data: &[$t], info: &cpal::InputCallbackInfo| {
                    let t_ns = info.timestamp().capture.as_nanos() as u64;
                    let frames = (data.len() / usize::from(channels)) as u32;
                    producer.push(t_ns, frames, data.iter().map($to_f32));
                },
                on_error(events),
                None,
            )
        };
    }
    match format {
        SampleFormat::F32 => build!(f32, |&s| s),
        SampleFormat::I16 => build!(i16, |&s| f32::from(s) / 32_768.0),
        SampleFormat::I32 => build!(i32, |&s| s as f32 / 2_147_483_648.0),
        other => Err(cpal::Error::with_message(
            ErrorKind::UnsupportedConfig,
            format!("formato {other:?} não suportado"),
        )),
    }
}

/// Silêncio na saída, para o loopback não parar quando nada toca.
fn silence(
    device: &cpal::Device,
    config: &StreamConfig,
    format: SampleFormat,
    events: Events,
) -> Result<Stream, cpal::Error> {
    macro_rules! build {
        ($t:ty, $zero:expr) => {
            device.build_output_stream(
                *config,
                |data: &mut [$t], _: &cpal::OutputCallbackInfo| data.fill($zero),
                on_error(events),
                None,
            )
        };
    }
    match format {
        SampleFormat::F32 => build!(f32, 0.0),
        SampleFormat::I16 => build!(i16, 0),
        SampleFormat::I32 => build!(i32, 0),
        other => Err(cpal::Error::with_message(
            ErrorKind::UnsupportedConfig,
            format!("formato {other:?} não suportado"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_pelo_cpal_ou_pelo_enumerador() {
        assert!(is_bluetooth(InterfaceType::Bluetooth, None));
        assert!(is_bluetooth(InterfaceType::Unknown, Some("BTHHFENUM")));
        assert!(is_bluetooth(InterfaceType::Unknown, Some("bthenum")));
        assert!(is_bluetooth(InterfaceType::Unknown, Some("BTHLEDEVICE")));
        assert!(!is_bluetooth(InterfaceType::Usb, Some("USB")));
        assert!(!is_bluetooth(InterfaceType::BuiltIn, Some("HDAUDIO")));
        assert!(!is_bluetooth(InterfaceType::Unknown, None));
    }
}
