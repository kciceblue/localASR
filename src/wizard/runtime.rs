//! Spawn a Tokio runtime on a background thread and expose its handle to
//! the (sync) eframe GUI loop. Async work scheduled via `Handle::spawn`
//! runs on the background runtime; results return through any preferred
//! sync channel (`oneshot::Receiver`, `std::sync::mpsc::Receiver`, etc.).

use std::sync::mpsc;
use std::thread;
use tokio::runtime::{Handle, Runtime};

pub struct WizardRuntime {
    pub handle: Handle,
    shutdown_tx: Option<mpsc::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}

impl WizardRuntime {
    pub fn start() -> anyhow::Result<Self> {
        let rt = Runtime::new()?;
        let handle = rt.handle().clone();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
        // Park the runtime in a dedicated thread so it stays alive for the
        // lifetime of the wizard. The worker blocks on the shutdown channel;
        // when Drop signals (or the sender drops), recv() returns and we
        // shut the runtime down with a short timeout so in-flight HTTP
        // doesn't hang us.
        let join = thread::Builder::new()
            .name("wizard-runtime".into())
            .spawn(move || {
                let _ = shutdown_rx.recv();
                rt.shutdown_timeout(std::time::Duration::from_secs(2));
            })?;
        Ok(Self {
            handle,
            shutdown_tx: Some(shutdown_tx),
            join: Some(join),
        })
    }
}

impl Drop for WizardRuntime {
    fn drop(&mut self) {
        // Best-effort: signal the worker thread to exit and join it.
        drop(self.shutdown_tx.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
