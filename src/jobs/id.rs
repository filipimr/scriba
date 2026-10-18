//! Identificador de job no formato ULID: 48 bits com os milissegundos desde 1970 e 80 bits
//! aleatórios, escritos em 26 caracteres Crockford base32. A ordem dos ids é a ordem de
//! criação, mesmo dentro do mesmo milissegundo.

use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(u128);

/// Último id gerado: no mesmo milissegundo (ou se o relógio voltar), o próximo é o
/// sucessor dele, para a ordem nunca inverter.
static LAST: Mutex<u128> = Mutex::new(0);

impl JobId {
    pub fn generate() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis())
            & ((1 << 48) - 1);
        let mut random = [0u8; 16];
        getrandom::fill(&mut random[6..]).expect("gerador aleatório do sistema");
        let candidate = (millis << 80) | u128::from_be_bytes(random);
        let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
        *last = candidate.max(*last + 1);
        Self(*last)
    }

    /// Milissegundos desde 1970 em que o id foi gerado.
    pub fn millis(self) -> u64 {
        (self.0 >> 80) as u64
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = (0..26)
            .rev()
            .map(|i| ALPHABET[((self.0 >> (i * 5)) & 31) as usize] as char)
            .collect();
        f.write_str(&text)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidId;

impl FromStr for JobId {
    type Err = InvalidId;

    /// Aceita maiúsculas e minúsculas; o primeiro caractere vai até 7 (128 bits).
    fn from_str(s: &str) -> Result<Self, InvalidId> {
        if s.len() != 26 {
            return Err(InvalidId);
        }
        s.bytes()
            .enumerate()
            .try_fold(0u128, |acc, (i, c)| {
                let digit = ALPHABET
                    .iter()
                    .position(|a| *a == c.to_ascii_uppercase())
                    .ok_or(InvalidId)?;
                if i == 0 && digit > 7 {
                    return Err(InvalidId);
                }
                Ok((acc << 5) | digit as u128)
            })
            .map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tem_26_caracteres_crockford() {
        let id = JobId::generate().to_string();
        assert_eq!(id.len(), 26);
        assert!(id.bytes().all(|c| ALPHABET.contains(&c)), "{id}");
    }

    #[test]
    fn ordem_segue_a_criacao() {
        let ids: Vec<JobId> = (0..1000).map(|_| JobId::generate()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        let texts: Vec<String> = ids.iter().map(ToString::to_string).collect();
        assert!(
            texts.windows(2).all(|w| w[0] < w[1]),
            "texto ordena como o id"
        );
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        assert!(now.as_millis() as u64 - ids[0].millis() < 60_000);
    }

    #[test]
    fn ida_e_volta_pelo_texto() {
        let id = JobId::generate();
        assert_eq!(id.to_string().parse(), Ok(id));
        assert_eq!(id.to_string().to_lowercase().parse(), Ok(id));
        let fixed = "01JAZ5Y3V8K9M2N4P6Q7R8S9T0";
        assert_eq!(fixed.parse::<JobId>().unwrap().to_string(), fixed);
    }

    #[test]
    fn texto_invalido_e_recusado() {
        for s in [
            "",
            "01JAZ5Y3V8K9M2N4P6Q7R8S9T",
            "01JAZ5Y3V8K9M2N4P6Q7R8S9TU",
            "81JAZ5Y3V8K9M2N4P6Q7R8S9T0",
            "../../../../windows/system",
        ] {
            assert_eq!(s.parse::<JobId>(), Err(InvalidId), "{s}");
        }
    }
}
