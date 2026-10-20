//! `IdleCache`: guarda um valor caro de criar (o modelo carregado) e o libera depois de um
//! tempo sem uso. Uma thread espera numa `Condvar` até o prazo e solta o valor se ninguém o
//! usou nesse meio-tempo.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct IdleCache<T: Send + 'static> {
    shared: Arc<Shared<T>>,
    reaper: Option<JoinHandle<()>>,
}

struct Shared<T> {
    slot: Mutex<Slot<T>>,
    wake: Condvar,
    idle: Duration,
    /// Lido sem o lock, que fica preso durante o uso.
    loaded: AtomicBool,
}

struct Slot<T> {
    value: Option<T>,
    last_used: Instant,
    stop: bool,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, Slot<T>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T: Send + 'static> IdleCache<T> {
    /// Libera o valor depois de `idle` sem uso.
    pub fn new(idle: Duration) -> Self {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                value: None,
                last_used: Instant::now(),
                stop: false,
            }),
            wake: Condvar::new(),
            idle,
            loaded: AtomicBool::new(false),
        });
        let reaper = thread::Builder::new()
            .name("scriba-cache".into())
            .spawn({
                let shared = shared.clone();
                move || reap(&shared)
            })
            .expect("criando a thread do cache");
        Self {
            shared,
            reaper: Some(reaper),
        }
    }

    /// Roda `f` com o valor, criado por `load` se ainda não existe. Um uso por vez: quem
    /// chamar durante outro uso espera. Se `load` falha, nada fica guardado.
    pub fn with<R, E>(
        &self,
        load: impl FnOnce() -> Result<T, E>,
        f: impl FnOnce(&mut T) -> R,
    ) -> Result<R, E> {
        let mut slot = self.shared.lock();
        let value = match &mut slot.value {
            Some(value) => value,
            empty => {
                let value = empty.insert(load()?);
                self.shared.loaded.store(true, Ordering::Relaxed);
                value
            }
        };
        let result = f(value);
        slot.last_used = Instant::now();
        drop(slot);
        self.shared.wake.notify_all();
        Ok(result)
    }

    pub fn is_loaded(&self) -> bool {
        self.shared.loaded.load(Ordering::Relaxed)
    }
}

/// A thread do cache: dorme até o prazo do último uso e solta o valor quando ele vence.
fn reap<T>(shared: &Shared<T>) {
    let mut slot = shared.lock();
    loop {
        if slot.stop {
            return;
        }
        if slot.value.is_none() {
            slot = shared
                .wake
                .wait(slot)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        let deadline = slot.last_used + shared.idle;
        let now = Instant::now();
        if now < deadline {
            slot = shared
                .wake
                .wait_timeout(slot, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        }
        let expired = slot.value.take();
        shared.loaded.store(false, Ordering::Relaxed);
        // Fora do lock: liberar um modelo grande leva tempo.
        drop(slot);
        drop(expired);
        slot = shared.lock();
    }
}

impl<T: Send + 'static> Drop for IdleCache<T> {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(reaper) = self.reaper.take() {
            let _ = reaper.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// Conta quantos valores foram criados e quantos já foram soltos.
    #[derive(Clone, Default)]
    struct Counter {
        loads: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    struct Value(Arc<AtomicUsize>);

    impl Drop for Value {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Counter {
        fn load(&self) -> Result<Value, ()> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(Value(self.drops.clone()))
        }

        fn loads(&self) -> usize {
            self.loads.load(Ordering::SeqCst)
        }

        fn drops(&self) -> usize {
            self.drops.load(Ordering::SeqCst)
        }
    }

    /// Espera a condição por até 5 s.
    fn eventually(cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        cond()
    }

    #[test]
    fn carrega_no_primeiro_uso_e_reaproveita() {
        let counter = Counter::default();
        let cache = IdleCache::new(Duration::from_secs(600));
        assert!(!cache.is_loaded());
        for i in 0..3 {
            let got = cache.with(|| counter.load(), |_| i * 2).unwrap();
            assert_eq!(got, i * 2);
        }
        assert_eq!(counter.loads(), 1);
        assert!(cache.is_loaded());
        drop(cache);
        assert_eq!(counter.drops(), 1, "o cache solta o valor ao sair");
    }

    #[test]
    fn libera_depois_do_tempo_ocioso_e_recarrega() {
        let counter = Counter::default();
        let cache = IdleCache::new(Duration::from_millis(50));
        cache.with(|| counter.load(), |_| ()).unwrap();
        assert!(eventually(|| counter.drops() == 1 && !cache.is_loaded()));
        cache.with(|| counter.load(), |_| ()).unwrap();
        assert_eq!(counter.loads(), 2);
        assert!(cache.is_loaded());
    }

    #[test]
    fn nao_libera_durante_o_uso() {
        let counter = Counter::default();
        let cache = IdleCache::new(Duration::from_millis(20));
        cache
            .with(
                || counter.load(),
                |_| {
                    thread::sleep(Duration::from_millis(200));
                    assert_eq!(counter.drops(), 0);
                },
            )
            .unwrap();
        // O prazo conta a partir do fim do uso.
        assert!(eventually(|| counter.drops() == 1));
    }

    #[test]
    fn uso_mantem_o_valor_vivo() {
        let counter = Counter::default();
        let cache = IdleCache::new(Duration::from_millis(500));
        for _ in 0..6 {
            cache.with(|| counter.load(), |_| ()).unwrap();
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(counter.loads(), 1);
        assert_eq!(counter.drops(), 0);
    }

    #[test]
    fn falha_ao_carregar_nao_guarda_nada() {
        let counter = Counter::default();
        let cache: IdleCache<Value> = IdleCache::new(Duration::from_secs(600));
        assert_eq!(
            cache.with(|| Err("sem memória"), |_| ()),
            Err("sem memória")
        );
        assert!(!cache.is_loaded());
        cache.with(|| counter.load(), |_| ()).unwrap();
        assert_eq!(counter.loads(), 1);
    }

    #[test]
    fn usos_simultaneos_esperam_a_vez() {
        let counter = Counter::default();
        let cache = Arc::new(IdleCache::new(Duration::from_secs(600)));
        let busy = Arc::new(AtomicBool::new(false));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (cache, counter, busy) = (cache.clone(), counter.clone(), busy.clone());
                thread::spawn(move || {
                    cache
                        .with(
                            || counter.load(),
                            |_| {
                                assert!(!busy.swap(true, Ordering::SeqCst), "dois usos juntos");
                                thread::sleep(Duration::from_millis(20));
                                busy.store(false, Ordering::SeqCst);
                            },
                        )
                        .unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(counter.loads(), 1);
    }
}
