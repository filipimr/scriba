//! O áudio extraído em disco (16 kHz mono f32 LE, cru), lido de volta por intervalos: a
//! transcrição em blocos não segura a duração inteira na memória (~230 MB por hora em f32).
//! O arquivo é apagado quando o `Spool` sai de cena, em qualquer desfecho do job.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::asr::Samples;

const SAMPLE_BYTES: usize = size_of::<f32>();

pub struct Spool {
    path: PathBuf,
    /// Só fica `None` no `Drop`, que fecha o arquivo antes de apagá-lo.
    file: Option<BufWriter<File>>,
    len: usize,
}

impl Spool {
    /// Cria (ou trunca) o arquivo em `path`.
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            path: path.into(),
            file: Some(BufWriter::with_capacity(1 << 20, file)),
            len: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn file(&mut self) -> &mut BufWriter<File> {
        self.file.as_mut().expect("aberto até o drop")
    }

    /// Acrescenta amostras ao fim.
    pub fn push(&mut self, samples: &[f32]) -> io::Result<()> {
        let file = self.file();
        for s in samples {
            file.write_all(&s.to_le_bytes())?;
        }
        self.len += samples.len();
        Ok(())
    }
}

impl Samples for Spool {
    fn len(&self) -> usize {
        self.len
    }

    fn read(&mut self, range: Range<usize>, buf: &mut Vec<f32>) -> io::Result<()> {
        let range = range.start.min(self.len)..range.end.min(self.len);
        buf.clear();
        buf.resize(range.len(), 0.0);
        let file = self.file();
        file.flush()?;
        let file = file.get_mut();
        file.seek(SeekFrom::Start((range.start * SAMPLE_BYTES) as u64))?;
        // SAFETY: qualquer padrão de bits é um f32 válido, e o slice de bytes cobre
        // exatamente a memória de `buf`.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * SAMPLE_BYTES)
        };
        file.read_exact(bytes)?;
        // O x64 é little-endian, como o arquivo: só por garantia em outra arquitetura.
        if cfg!(target_endian = "big") {
            for s in buf.iter_mut() {
                *s = f32::from_le_bytes(s.to_ne_bytes());
            }
        }
        file.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        // O arquivo precisa estar fechado para o Windows apagá-lo; o que falta gravar não
        // interessa mais.
        drop(self.file.take().map(BufWriter::into_parts));
        if let Err(e) = fs::remove_file(&self.path)
            && e.kind() != io::ErrorKind::NotFound
        {
            eprintln!("media: não apaguei {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_de_volta_o_que_gravou_e_apaga_no_fim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.f32");
        let mut spool = Spool::create(&path).unwrap();
        let samples: Vec<f32> = (0..10_000).map(|i| i as f32 * 0.5 - 7.25).collect();
        spool.push(&samples[..4_000]).unwrap();
        spool.push(&samples[4_000..]).unwrap();
        assert_eq!(Samples::len(&spool), 10_000);

        let mut buf = vec![1.0; 3];
        spool.read(2_500..7_500, &mut buf).unwrap();
        assert_eq!(buf, samples[2_500..7_500]);
        // Depois de ler, continua gravando no fim.
        spool.push(&[42.0]).unwrap();
        spool.read(9_998..20_000, &mut buf).unwrap();
        assert_eq!(buf, [samples[9_998], samples[9_999], 42.0]);
        spool.read(30_000..40_000, &mut buf).unwrap();
        assert!(buf.is_empty());

        assert!(path.is_file());
        drop(spool);
        assert!(!path.exists());
    }

    #[test]
    fn pasta_que_nao_existe_da_erro() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Spool::create(&dir.path().join("nada/a.f32")).is_err());
    }
}
