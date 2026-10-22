//! Hora local do Windows (`GetLocalTime`), com o fuso calculado pela diferença para a hora
//! UTC (`GetSystemTime`). Serve ao nome da pasta da saída, ao cabeçalho da transcrição e
//! ao `job.json`.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};

/// Também vai para o `gravacao.json`, de onde uma gravação retomada recupera a hora.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Diferença para UTC, em minutos (−180 em Brasília).
    pub offset_minutes: i16,
}

impl LocalTime {
    pub fn now() -> Self {
        let mut local = empty();
        let mut utc = empty();
        // SAFETY: as duas funções só escrevem na estrutura recebida.
        unsafe {
            GetLocalTime(&mut local);
            GetSystemTime(&mut utc);
        }
        let mut time = from_system(&local);
        time.offset_minutes =
            offset_minutes(seconds(&from_system(&local)), seconds(&from_system(&utc)));
        time
    }

    /// `2026-10-19T14:30:05-03:00`.
    pub fn rfc3339(&self) -> String {
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let offset = self.offset_minutes.unsigned_abs();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            offset / 60,
            offset % 60
        )
    }

    /// O inverso de [`rfc3339`](Self::rfc3339): `2026-10-19T14:30:05-03:00` (também `Z`, e
    /// frações de segundo, ignoradas). `None` se o texto não é uma data válida.
    pub fn parse_rfc3339(s: &str) -> Option<Self> {
        let num = |range: std::ops::Range<usize>| -> Option<u16> {
            let part = s.get(range)?;
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        let sep = |i: usize, c: u8| s.as_bytes().get(i) == Some(&c);
        if !(sep(4, b'-') && sep(7, b'-') && matches!(s.as_bytes().get(10), Some(b'T' | b't'))) {
            return None;
        }
        if !(sep(13, b':') && sep(16, b':')) {
            return None;
        }
        let time = Self {
            year: num(0..4)?,
            month: num(5..7)? as u8,
            day: num(8..10)? as u8,
            hour: num(11..13)? as u8,
            minute: num(14..16)? as u8,
            second: num(17..19)? as u8,
            offset_minutes: 0,
        };
        let mut rest = &s[19..];
        if let Some(frac) = rest.strip_prefix('.') {
            let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return None;
            }
            rest = &frac[digits..];
        }
        let offset_minutes = match rest.as_bytes() {
            [b'Z' | b'z'] => 0,
            [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
                let digits = [*h1, *h2, *m1, *m2];
                if !digits.iter().all(u8::is_ascii_digit) {
                    return None;
                }
                let d = |b: u8| i16::from(b - b'0');
                let (h, m) = (d(*h1) * 10 + d(*h2), d(*m1) * 10 + d(*m2));
                if h > 23 || m > 59 {
                    return None;
                }
                let minutes = h * 60 + m;
                if *sign == b'-' { -minutes } else { minutes }
            }
            _ => return None,
        };
        let valid = (1..=12).contains(&time.month)
            && time.day >= 1
            && time.day <= days_in_month(time.year, time.month)
            && time.hour < 24
            && time.minute < 60
            && time.second < 61;
        valid.then_some(Self {
            offset_minutes,
            ..time
        })
    }

    /// Segundos desde 1970 em UTC, para ordenar horas de fusos diferentes.
    pub fn unix_seconds(&self) -> i64 {
        seconds(self) - i64::from(self.offset_minutes) * 60
    }

    /// `2026-10-19 1430`, o início do nome da pasta.
    pub fn folder_stamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }

    /// `19/10/2026 14:30`, para quem lê.
    pub fn display(&self) -> String {
        format!(
            "{:02}/{:02}/{:04} {:02}:{:02}",
            self.day, self.month, self.year, self.hour, self.minute
        )
    }
}

fn empty() -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    }
}

fn from_system(t: &SYSTEMTIME) -> LocalTime {
    LocalTime {
        year: t.wYear,
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
        offset_minutes: 0,
    }
}

/// Segundos desde 1970 da data e hora, sem fuso.
fn seconds(t: &LocalTime) -> i64 {
    let days = days_from_civil(i64::from(t.year), i64::from(t.month), i64::from(t.day));
    days * 86_400 + i64::from(t.hour) * 3_600 + i64::from(t.minute) * 60 + i64::from(t.second)
}

/// Dias desde 1970-01-01 no calendário gregoriano (algoritmo de Howard Hinnant).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A hora do sistema, atrás de uma trait: a expiração do áudio e a limpeza por idade são
/// testadas com o [`FakeClock`].
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Um relógio parado, que só anda com [`advance`](Self::advance).
#[cfg(any(test, feature = "test-support"))]
pub struct FakeClock(std::sync::Mutex<SystemTime>);

#[cfg(any(test, feature = "test-support"))]
impl FakeClock {
    pub fn new(now: SystemTime) -> Self {
        Self(std::sync::Mutex::new(now))
    }

    pub fn advance(&self, by: std::time::Duration) {
        *self.0.lock().unwrap() += by;
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }
}

/// Fuso em minutos, arredondado a 15 min: as duas leituras do relógio podem cair em
/// segundos diferentes.
fn offset_minutes(local: i64, utc: i64) -> i16 {
    let quarters = ((local - utc) as f64 / 900.0).round() as i64;
    (quarters * 15) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> LocalTime {
        LocalTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            offset_minutes: 0,
        }
    }

    #[test]
    fn dias_desde_1970() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 10, 19), 20_745);
    }

    #[test]
    fn fuso_pela_diferenca() {
        let local = seconds(&at(2026, 10, 19, 21, 30, 0));
        let utc = seconds(&at(2026, 10, 20, 0, 30, 1));
        assert_eq!(offset_minutes(local, utc), -180);
        let local = seconds(&at(2026, 1, 1, 5, 45, 59));
        let utc = seconds(&at(2026, 1, 1, 0, 0, 0));
        assert_eq!(offset_minutes(local, utc), 345);
    }

    #[test]
    fn formatos() {
        let t = LocalTime {
            offset_minutes: -180,
            ..at(2026, 10, 19, 9, 5, 7)
        };
        assert_eq!(t.rfc3339(), "2026-10-19T09:05:07-03:00");
        assert_eq!(t.folder_stamp(), "2026-10-19 0905");
        assert_eq!(t.display(), "19/10/2026 09:05");
        let t = LocalTime {
            offset_minutes: 330,
            ..t
        };
        assert_eq!(t.rfc3339(), "2026-10-19T09:05:07+05:30");
    }

    #[test]
    fn le_rfc3339() {
        let t = LocalTime::parse_rfc3339("2026-10-19T09:05:07-03:00").unwrap();
        assert_eq!(
            t,
            LocalTime {
                offset_minutes: -180,
                ..at(2026, 10, 19, 9, 5, 7)
            }
        );
        assert_eq!(
            LocalTime::parse_rfc3339("2024-02-29T23:59:59.123Z"),
            Some(at(2024, 2, 29, 23, 59, 59))
        );
        for bad in [
            "",
            "2026-10-19",
            "2026-10-19T09:05:07",
            "2026-13-01T00:00:00Z",
            "2025-02-29T00:00:00Z",
            "2026-10-19T24:00:00Z",
            "2026-10-19T09:05:07+3:00",
            "2026-10-19T09:05:07-03:00x",
            "2026-10-19 09:05:07Z",
            "2026-1é-19T09:05:07Z",
            "+026-10-19T09:05:07Z",
        ] {
            assert_eq!(LocalTime::parse_rfc3339(bad), None, "{bad}");
        }
        // A mesma hora em fusos diferentes.
        let brt = LocalTime::parse_rfc3339("2026-10-19T21:00:00-03:00").unwrap();
        let utc = LocalTime::parse_rfc3339("2026-10-20T00:00:00Z").unwrap();
        assert_eq!(brt.unix_seconds(), utc.unix_seconds());
    }

    #[test]
    fn relogio_falso_so_anda_quando_pedido() {
        let start = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let clock = FakeClock::new(start);
        assert_eq!(clock.now(), start);
        clock.advance(std::time::Duration::from_secs(60));
        assert_eq!(clock.now(), start + std::time::Duration::from_secs(60));
        assert!(SystemClock.now() > start);
    }

    proptest::proptest! {
        #[test]
        fn rfc3339_ida_e_volta(
            year in 0u16..10_000,
            month in 1u8..=12,
            day in 1u8..=31,
            hour in 0u8..24,
            minute in 0u8..60,
            second in 0u8..60,
            offset in -(23 * 60 + 59)..=(23 * 60 + 59i16),
        ) {
            let t = LocalTime {
                day: day.min(days_in_month(year, month)),
                offset_minutes: offset,
                ..at(year, month, 1, hour, minute, second)
            };
            proptest::prop_assert_eq!(LocalTime::parse_rfc3339(&t.rfc3339()), Some(t));
        }
    }

    #[test]
    fn agora_e_plausivel() {
        let t = LocalTime::now();
        assert!(t.year >= 2026 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day));
        assert!((-720..=840).contains(&t.offset_minutes));
    }
}
