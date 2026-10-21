//! Hora local do Windows (`GetLocalTime`), com o fuso calculado pela diferença para a hora
//! UTC (`GetSystemTime`). Serve ao nome da pasta da saída, ao cabeçalho da transcrição e
//! ao `job.json`.

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
    fn agora_e_plausivel() {
        let t = LocalTime::now();
        assert!(t.year >= 2026 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day));
        assert!((-720..=840).contains(&t.offset_minutes));
    }
}
