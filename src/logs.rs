//! Logs do app em `<data-dir>\logs\scriba.log`, com rotação por tamanho.
//!
//! A captura é feita nos handles do processo: o stdout e o stderr, do Rust e do CRT (onde
//! escrevem o whisper.cpp, o ggml e o FFmpeg), passam por dois pipes. Uma thread para cada um
//! grava as linhas no arquivo, com data e hora, e as repete no destino de antes (o console ou
//! o pipe de quem abriu o exe), se havia um. Nenhuma chamada de `eprintln!` muda. O panic
//! hook e o filtro de falhas nativas escrevem direto no arquivo, sem passar pelo pipe, para a
//! linha não se perder se o processo morrer logo depois.

use std::ffi::{c_char, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    WriteConsoleW,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Tamanho a partir do qual o `scriba.log` gira.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Quantos arquivos antigos ficam (`scriba.1.log` … `scriba.4.log`): até 25 MB no total.
pub const KEEP: usize = 4;
const BASE: &str = "scriba";
/// Uma linha sem quebra maior que isto é gravada em pedaços, para a memória não crescer.
const MAX_LINE: usize = 64 * 1024;
/// O pipe do stdout e o do stderr; o padrão do Windows (4 KB) travaria quem escreve antes.
const PIPE_BYTES: u32 = 64 * 1024;
/// Quanto o encerramento espera as threads gravarem o que ainda estava no pipe.
const DRAIN: Duration = Duration::from_secs(2);

/// O `scriba.log`, que gira para `scriba.1.log` ao passar de `max_bytes`.
pub struct RotatingFile {
    dir: PathBuf,
    max_bytes: u64,
    keep: usize,
    file: Option<File>,
    size: u64,
}

impl RotatingFile {
    /// Abre (ou cria) o arquivo atual em `dir`; um arquivo que já passou do limite gira
    /// antes da primeira linha.
    pub fn open(dir: &Path, max_bytes: u64, keep: usize) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let mut log = Self {
            dir: dir.to_path_buf(),
            max_bytes,
            keep,
            file: None,
            size: 0,
        };
        log.reopen()?;
        if log.size >= max_bytes {
            log.rotate()?;
        }
        Ok(log)
    }

    /// O arquivo atual em `dir`.
    pub fn current(dir: &Path) -> PathBuf {
        dir.join(format!("{BASE}.log"))
    }

    /// O `n`-ésimo arquivo antigo (1 é o mais novo).
    pub fn old(dir: &Path, n: usize) -> PathBuf {
        dir.join(format!("{BASE}.{n}.log"))
    }

    /// Grava `line` com `stamp` na frente e CRLF no fim; gira antes se o arquivo passaria do
    /// limite.
    pub fn write_line(&mut self, stamp: &str, line: &[u8]) -> io::Result<()> {
        let mut buf = Vec::with_capacity(stamp.len() + line.len() + 3);
        buf.extend_from_slice(stamp.as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(line);
        buf.extend_from_slice(b"\r\n");
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.reopen()?;
        }
        let file = self.file.as_mut().expect("aberto acima");
        file.write_all(&buf)?;
        self.size += buf.len() as u64;
        Ok(())
    }

    fn reopen(&mut self) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(Self::current(&self.dir))?;
        self.size = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    /// `scriba.3.log` → `scriba.4.log`, …, `scriba.log` → `scriba.1.log`, e um atual novo.
    fn rotate(&mut self) -> io::Result<()> {
        // O Windows não renomeia um arquivo aberto por nós.
        self.file = None;
        let result = (|| {
            remove_if_exists(&Self::old(&self.dir, self.keep))?;
            for n in (1..self.keep).rev() {
                rename_if_exists(&Self::old(&self.dir, n), &Self::old(&self.dir, n + 1))?;
            }
            if self.keep == 0 {
                remove_if_exists(&Self::current(&self.dir))
            } else {
                rename_if_exists(&Self::current(&self.dir), &Self::old(&self.dir, 1))
            }
        })();
        self.reopen()?;
        if result.is_err() {
            // Outro programa segura um dos arquivos: o atual continua crescendo e a próxima
            // tentativa fica para depois de mais `max_bytes`.
            self.size = 0;
        }
        result
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Junta os pedaços lidos do pipe em linhas, sem o `\r` do fim.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    fn push(&mut self, chunk: &[u8], mut each: impl FnMut(&[u8])) {
        let mut rest = chunk;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            self.partial.extend_from_slice(&rest[..i]);
            each(trim_cr(&self.partial));
            self.partial.clear();
            rest = &rest[i + 1..];
        }
        self.partial.extend_from_slice(rest);
        if self.partial.len() >= MAX_LINE {
            each(&self.partial);
            self.partial.clear();
        }
    }

    /// O que sobrou sem quebra de linha no fim do pipe.
    fn finish(&mut self, mut each: impl FnMut(&[u8])) {
        if !self.partial.is_empty() {
            each(trim_cr(&self.partial));
            self.partial.clear();
        }
    }
}

fn trim_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// `2026-10-21 14:30:05.123`, a hora local de agora.
pub fn stamp() -> String {
    // SAFETY: estrutura C sem invariantes; o GetLocalTime só escreve nela.
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format_stamp(
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        t.wMilliseconds,
    )
}

fn format_stamp(year: u16, month: u16, day: u16, h: u16, m: u16, s: u16, ms: u16) -> String {
    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Um handle de saída (o console ou o pipe de quem abriu o exe); 0 = nenhum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target(usize);

impl Target {
    const NONE: Self = Self(0);

    /// O handle padrão `id` de agora, se ele é válido.
    fn current(id: STD_HANDLE) -> Self {
        // SAFETY: só lê a tabela de handles padrão do processo.
        let h = unsafe { GetStdHandle(id) };
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            Self::NONE
        } else {
            Self(h as usize)
        }
    }

    /// Uma cópia própria do handle padrão `id`: o CRT pode fechar o dele ao trocar o fd.
    fn duplicate(id: STD_HANDLE) -> Self {
        match Self::current(id) {
            Self::NONE => Self::NONE,
            Self(h) => duplicate(h as HANDLE).map_or(Self::NONE, |h| Self(h as usize)),
        }
    }

    fn handle(self) -> HANDLE {
        self.0 as HANDLE
    }

    /// Repete a linha no destino; no console, em UTF-16, para os acentos saírem certos.
    fn write_line(self, line: &[u8]) {
        if self == Self::NONE {
            return;
        }
        let h = self.handle();
        let mut mode = 0;
        // SAFETY: `h` é um handle aberto deste processo; `mode` é só saída.
        if unsafe { GetConsoleMode(h, &mut mode) } != 0 {
            let text: Vec<u16> = String::from_utf8_lossy(line)
                .encode_utf16()
                .chain("\n".encode_utf16())
                .collect();
            let mut written = 0;
            // SAFETY: `text` vive durante a chamada e o tamanho é o dele.
            unsafe {
                WriteConsoleW(
                    h,
                    text.as_ptr(),
                    text.len() as u32,
                    &mut written,
                    std::ptr::null(),
                )
            };
        } else {
            let mut buf = Vec::with_capacity(line.len() + 1);
            buf.extend_from_slice(line);
            buf.push(b'\n');
            write_handle(h, &buf);
        }
    }
}

fn duplicate(h: HANDLE) -> io::Result<HANDLE> {
    let mut copy = std::ptr::null_mut();
    // SAFETY: `h` é um handle aberto deste processo; a cópia é nossa.
    let ok = unsafe {
        let me = GetCurrentProcess();
        DuplicateHandle(me, h, me, &mut copy, 0, 0, DUPLICATE_SAME_ACCESS)
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(copy)
    }
}

fn write_handle(h: HANDLE, mut buf: &[u8]) {
    while !buf.is_empty() {
        let mut written = 0;
        // SAFETY: `buf` vive durante a chamada; sem OVERLAPPED, a escrita é síncrona.
        let ok = unsafe {
            WriteFile(
                h,
                buf.as_ptr(),
                buf.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || written == 0 {
            return;
        }
        buf = &buf[written as usize..];
    }
}

/// O arquivo, dividido entre as threads dos pipes, o panic hook e o filtro de falhas.
struct Shared {
    file: Mutex<RotatingFile>,
    /// Onde o stderr escrevia antes dos logs: o panic hook também o repete lá.
    stderr: Target,
}

impl Shared {
    /// Uma linha que veio do pipe.
    fn line(&self, line: &[u8], target: Target) {
        let stamp = stamp();
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(e) = file.write_line(&stamp, line) {
            drop(file);
            self.stderr
                .write_line(format!("não consegui gravar o log: {e}").as_bytes());
        }
        target.write_line(line);
    }

    /// Um texto do panic hook ou do filtro de falhas: direto no arquivo, sem esperar pela
    /// thread do pipe. Se o arquivo continua ocupado (o pânico foi dentro de um log), fica
    /// só no stderr.
    fn direct(&self, text: &str) {
        let stamp = stamp();
        let deadline = Instant::now() + Duration::from_millis(200);
        loop {
            match self.file.try_lock() {
                Ok(mut file) => {
                    for line in text.lines() {
                        let _ = file.write_line(&stamp, line.as_bytes());
                    }
                    break;
                }
                Err(TryLockError::Poisoned(p)) => {
                    let mut file = p.into_inner();
                    for line in text.lines() {
                        let _ = file.write_line(&stamp, line.as_bytes());
                    }
                    break;
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(TryLockError::WouldBlock) => break,
            }
        }
        for line in text.lines() {
            self.stderr.write_line(line.as_bytes());
        }
    }
}

// O CRT estático (o mesmo do whisper.cpp e do FFmpeg, todos dentro do exe).
unsafe extern "C" {
    fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
    fn _dup2(fd1: i32, fd2: i32) -> i32;
    fn _close(fd: i32) -> i32;
    fn __acrt_iob_func(index: u32) -> *mut c_void;
    fn setvbuf(stream: *mut c_void, buffer: *mut c_char, mode: i32, size: usize) -> i32;
}

const O_WRONLY: i32 = 0x0001;
const O_BINARY: i32 = 0x8000;
const IONBF: i32 = 0x0004;

/// Liga o fd `fd` do CRT a uma cópia de `h`.
///
/// # Safety
///
/// `h` precisa ser um handle aberto deste processo.
unsafe fn crt_point(fd: i32, h: HANDLE) -> io::Result<()> {
    let copy = duplicate(h)?;
    // SAFETY: o `_open_osfhandle` fica com a cópia; o `_dup2` faz a dele para o `fd`.
    unsafe {
        let tmp = _open_osfhandle(copy as isize, O_WRONLY | O_BINARY);
        if tmp < 0 {
            CloseHandle(copy);
            return Err(io::Error::other("_open_osfhandle falhou"));
        }
        let result = _dup2(tmp, fd);
        _close(tmp);
        if result != 0 {
            return Err(io::Error::other("_dup2 falhou"));
        }
    }
    Ok(())
}

/// O stdout ou o stderr desviado para um pipe, com a thread que o lê.
struct Stream {
    id: STD_HANDLE,
    fd: i32,
    /// O destino de antes, que volta no fim.
    target: Target,
    /// A ponta de escrita do pipe, que é o handle padrão enquanto o log vive.
    write: usize,
    reader: Option<JoinHandle<()>>,
}

impl Stream {
    fn redirect(
        id: STD_HANDLE,
        fd: i32,
        target: Target,
        name: &str,
        shared: Arc<Shared>,
    ) -> io::Result<Self> {
        let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: os dois handles são só saída; sem atributos, o pipe não é herdável.
        if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), PIPE_BYTES) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `write` acabou de ser aberto.
        if let Err(e) = unsafe { crt_point(fd, write) } {
            // SAFETY: os dois handles são nossos e ninguém mais os usa.
            unsafe {
                CloseHandle(read);
                CloseHandle(write);
            }
            return Err(e);
        }
        // O `_dup2` mexe no handle padrão: este vem depois.
        // SAFETY: `write` fica aberto até o `drop`.
        unsafe { SetStdHandle(id, write) };
        if fd == 1 {
            // O stdout do C (num pipe, com buffer cheio) sai na hora, como o do Rust.
            // SAFETY: o stream 1 do CRT existe pelo processo inteiro.
            unsafe { setvbuf(__acrt_iob_func(1), std::ptr::null_mut(), IONBF, 0) };
        }
        let read = read as usize;
        let reader = thread::Builder::new()
            .name(name.into())
            .spawn(move || pump(read as HANDLE, target, &shared))?;
        Ok(Self {
            id,
            fd,
            target,
            write: write as usize,
            reader: Some(reader),
        })
    }
}

impl Drop for Stream {
    /// O destino de antes volta; o pipe fecha e a thread grava o que faltava.
    fn drop(&mut self) {
        // SAFETY: os handles são deste processo; o `target` não é fechado, porque volta a
        // ser o handle padrão.
        unsafe {
            if self.target == Target::NONE || crt_point(self.fd, self.target.handle()).is_err() {
                _close(self.fd);
            }
            SetStdHandle(self.id, self.target.handle());
            CloseHandle(self.write as HANDLE);
        }
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + DRAIN;
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}

/// Lê o pipe até ele fechar.
fn pump(read: HANDLE, target: Target, shared: &Shared) {
    let mut buf = vec![0u8; 8192];
    let mut lines = Lines::default();
    loop {
        let mut n = 0;
        // SAFETY: `buf` vive durante a chamada; sem OVERLAPPED, a leitura é síncrona.
        let ok = unsafe {
            ReadFile(
                read,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || n == 0 {
            break;
        }
        lines.push(&buf[..n as usize], |line| shared.line(line, target));
    }
    lines.finish(|line| shared.line(line, target));
    // SAFETY: a ponta de leitura é só desta thread.
    unsafe { CloseHandle(read) };
}

/// O arquivo de log de agora, para o filtro de falhas nativas (uma função C sem contexto).
static ACTIVE: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

/// Os logs ligados; ao sair do escopo, o stdout e o stderr voltam ao que eram.
pub struct Logs {
    dir: PathBuf,
    streams: Vec<Stream>,
}

impl Logs {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// O arquivo atual.
    pub fn file(&self) -> PathBuf {
        RotatingFile::current(&self.dir)
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        self.streams.clear();
        let _ = std::panic::take_hook();
        *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Liga os logs em `dir`: o stdout e o stderr passam pelo arquivo, e o panic hook e o filtro
/// de falhas nativas gravam lá.
pub fn init(dir: &Path) -> io::Result<Logs> {
    let file = RotatingFile::open(dir, MAX_BYTES, KEEP)?;
    let out = Target::duplicate(STD_OUTPUT_HANDLE);
    let err = Target::duplicate(STD_ERROR_HANDLE);
    let shared = Arc::new(Shared {
        file: Mutex::new(file),
        stderr: err,
    });
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    // O `Vec` solta na ordem: no fim, o stdout volta antes do stderr.
    let streams = vec![
        Stream::redirect(STD_OUTPUT_HANDLE, 1, out, "scriba-log-out", shared.clone())?,
        Stream::redirect(STD_ERROR_HANDLE, 2, err, "scriba-log-err", shared.clone())?,
    ];
    install_panic_hook(shared.clone());
    *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) = Some(shared);
    // SAFETY: a função vale pelo processo inteiro.
    unsafe { SetUnhandledExceptionFilter(Some(native_crash)) };
    Ok(Logs {
        dir: dir.to_path_buf(),
        streams,
    })
}

/// Alguém vê o stderr (um console ou o pipe de quem abriu o exe)? Num app aberto pelo
/// Explorer, não: o erro fatal precisa de uma janela.
pub fn stderr_visible() -> bool {
    Target::current(STD_ERROR_HANDLE) != Target::NONE
}

fn install_panic_hook(shared: Arc<Shared>) {
    std::panic::set_hook(Box::new(move |info| {
        let thread = thread::current();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "?".into());
        let message = info.payload_as_str().unwrap_or("(sem mensagem)");
        let backtrace = std::backtrace::Backtrace::force_capture();
        shared.direct(&panic_text(
            thread.name().unwrap_or("sem nome"),
            &location,
            message,
            &backtrace.to_string(),
        ));
    }));
}

fn panic_text(thread: &str, location: &str, message: &str, backtrace: &str) -> String {
    format!("pânico na thread '{thread}' em {location}: {message}\nbacktrace:\n{backtrace}")
}

fn native_text(code: i32, address: *const c_void) -> String {
    format!("falha nativa: código 0x{:08X} em {address:p}", code as u32)
}

/// Uma falha nativa que ninguém tratou (whisper.cpp, FFmpeg): registra e deixa o Windows
/// seguir (o relatório de erro e o fim do processo).
unsafe extern "system" fn native_crash(info: *const EXCEPTION_POINTERS) -> i32 {
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
    // SAFETY: o Windows passa ponteiros válidos durante a chamada.
    let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    if let Some(record) = record
        && let Ok(active) = ACTIVE.try_lock()
        && let Some(shared) = active.as_ref()
    {
        shared.direct(&native_text(record.ExceptionCode, record.ExceptionAddress));
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// `Windows 11 25H2 (build 26300.7171)`, do registro. O `ProductName` diz "Windows 10" também
/// no 11, por isso a versão vem da build.
pub fn windows_version() -> String {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = reg_string(KEY, "CurrentBuild").unwrap_or_default();
    let display = reg_string(KEY, "DisplayVersion").unwrap_or_default();
    let ubr = reg_dword(KEY, "UBR");
    describe_windows(&build, &display, ubr)
}

fn describe_windows(build: &str, display: &str, ubr: Option<u32>) -> String {
    let major = match build.parse::<u32>() {
        Ok(b) if b >= 22_000 => "Windows 11",
        Ok(_) => "Windows 10",
        Err(_) => return "Windows (versão desconhecida)".into(),
    };
    let display = if display.is_empty() {
        String::new()
    } else {
        format!(" {display}")
    };
    let ubr = ubr.map(|u| format!(".{u}")).unwrap_or_default();
    format!("{major}{display} (build {build}{ubr})")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn reg_string(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buf = [0u16; 128];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: as strings terminam em zero e `size` é o tamanho de `buf` em bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn reg_dword(key: &str, value: &str) -> Option<u32> {
    let (key, value) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: as strings terminam em zero e `data` tem os 4 bytes informados.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (status == 0).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Os testes que desviam o stdout e o stderr do processo não rodam juntos (com o
    /// `cargo test`, que usa threads; o nextest já separa por processo).
    static CAPTURE: Mutex<()> = Mutex::new(());

    fn read(path: &Path) -> String {
        String::from_utf8_lossy(&fs::read(path).unwrap_or_default()).into_owned()
    }

    #[test]
    fn gira_ao_passar_do_limite_e_guarda_so_os_antigos_pedidos() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = RotatingFile::open(dir.path(), 100, 2).unwrap();
        // Cada linha tem 23 + 1 + 20 + 2 = 46 bytes: duas por arquivo.
        for i in 0..7 {
            log.write_line(
                "2026-10-21 14:30:05.123",
                format!("linha {i:014}").as_bytes(),
            )
            .unwrap();
        }
        let current = read(&RotatingFile::current(dir.path()));
        let first = read(&RotatingFile::old(dir.path(), 1));
        let second = read(&RotatingFile::old(dir.path(), 2));
        assert!(current.contains("linha 00000000000006"), "{current}");
        assert!(first.contains("linha 00000000000004") && first.contains("linha 00000000000005"));
        assert!(second.contains("linha 00000000000002"));
        assert!(!RotatingFile::old(dir.path(), 3).exists());
        assert!(current.ends_with("\r\n") && current.starts_with("2026-10-21 14:30:05.123 "));
    }

    #[test]
    fn arquivo_grande_de_antes_gira_na_abertura() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(RotatingFile::current(dir.path()), vec![b'x'; 150]).unwrap();
        let mut log = RotatingFile::open(dir.path(), 100, 1).unwrap();
        log.write_line("t", b"nova").unwrap();
        assert_eq!(read(&RotatingFile::current(dir.path())), "t nova\r\n");
        assert_eq!(
            fs::metadata(RotatingFile::old(dir.path(), 1))
                .unwrap()
                .len(),
            150
        );
    }

    #[test]
    fn sem_antigos_o_atual_recomeca() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = RotatingFile::open(dir.path(), 10, 0).unwrap();
        log.write_line("t", b"primeira").unwrap();
        log.write_line("t", b"segunda").unwrap();
        assert_eq!(read(&RotatingFile::current(dir.path())), "t segunda\r\n");
        assert!(!RotatingFile::old(dir.path(), 1).exists());
    }

    #[test]
    fn linhas_atravessam_os_pedacos_do_pipe() {
        let mut lines = Lines::default();
        let mut got = Vec::new();
        let mut each = |l: &[u8]| got.push(String::from_utf8(l.to_vec()).unwrap());
        lines.push(b"um\r\ndo", &mut each);
        lines.push(b"is\n\ntr", &mut each);
        lines.finish(&mut each);
        assert_eq!(got, ["um", "dois", "", "tr"]);
    }

    #[test]
    fn linha_enorme_sem_quebra_sai_em_pedacos() {
        let mut lines = Lines::default();
        let mut sizes = Vec::new();
        lines.push(&vec![b'a'; MAX_LINE + 10], |l| sizes.push(l.len()));
        assert_eq!(sizes, [MAX_LINE + 10]);
        lines.finish(|l| sizes.push(l.len()));
        assert_eq!(sizes.len(), 1);
    }

    #[test]
    fn formato_da_hora() {
        assert_eq!(
            format_stamp(2026, 1, 2, 3, 4, 5, 6),
            "2026-01-02 03:04:05.006"
        );
        let now = stamp();
        assert_eq!(now.len(), 23, "{now}");
        assert!(now.starts_with("20"), "{now}");
    }

    #[test]
    fn textos_do_panico_e_da_falha_nativa() {
        let text = panic_text("scriba-worker", "src/x.rs:1:2", "boom", "0: main");
        assert_eq!(
            text,
            "pânico na thread 'scriba-worker' em src/x.rs:1:2: boom\nbacktrace:\n0: main"
        );
        let text = native_text(0xC000_0005_u32 as i32, 0x1234 as *const c_void);
        assert_eq!(text, "falha nativa: código 0xC0000005 em 0x1234");
    }

    #[test]
    fn versao_do_windows() {
        assert_eq!(
            describe_windows("26300", "25H2", Some(7171)),
            "Windows 11 25H2 (build 26300.7171)"
        );
        assert_eq!(
            describe_windows("19045", "", None),
            "Windows 10 (build 19045)"
        );
        assert_eq!(
            describe_windows("", "", None),
            "Windows (versão desconhecida)"
        );
        assert!(
            windows_version().starts_with("Windows 1"),
            "{}",
            windows_version()
        );
    }

    #[test]
    fn stdout_e_stderr_do_rust_e_do_crt_vao_para_o_arquivo() {
        let _serial = CAPTURE.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let logs = init(dir.path()).unwrap();
        // Direto nos streams: as macros `print!` dos testes são capturadas pelo libtest.
        writeln!(io::stderr(), "do rust no stderr: ação").unwrap();
        writeln!(io::stdout(), "do rust no stdout").unwrap();
        unsafe extern "C" {
            fn _write(fd: i32, buf: *const c_void, count: u32) -> i32;
        }
        let c = b"do CRT no fd 2\n";
        // SAFETY: `c` vive durante a chamada; o fd 2 aponta para o pipe.
        assert_eq!(
            unsafe { _write(2, c.as_ptr().cast(), c.len() as u32) },
            c.len() as i32
        );
        let file = logs.file();
        drop(logs);
        let text = read(&file);
        for expected in [
            "do rust no stderr: ação",
            "do rust no stdout",
            "do CRT no fd 2",
        ] {
            assert!(text.contains(expected), "faltou {expected:?} em:\n{text}");
        }
        // Os handles voltaram: escrever depois não vai para o arquivo nem trava.
        writeln!(io::stderr(), "depois do fim").unwrap();
        assert!(!read(&file).contains("depois do fim"));
    }

    #[test]
    fn panico_grava_a_thread_o_lugar_e_a_mensagem() {
        let _serial = CAPTURE.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let logs = init(dir.path()).unwrap();
        let joined = thread::Builder::new()
            .name("teste-panico".into())
            .spawn(|| panic!("pânico de propósito"))
            .unwrap()
            .join();
        assert!(joined.is_err());
        let file = logs.file();
        drop(logs);
        let text = read(&file);
        assert!(
            text.contains("pânico na thread 'teste-panico' em src")
                && text.contains("logs.rs:")
                && text.contains("pânico de propósito")
                && text.contains("backtrace:"),
            "{text}"
        );
    }
}
