//! Spawn a Tokio runtime on a background thread and expose its handle to
//! the (sync) eframe GUI loop. Async work scheduled via `Handle::spawn`
//! runs on the background runtime; results return through any preferred
//! sync channel (`oneshot::Receiver`, `std::sync::mpsc::Receiver`, etc.).

use std::thread;
use tokio::runtime::{Handle, Runtime};

pub struct WizardRuntime {
    pub handle: Handle,
    _thread: thread::JoinHandle<()>,
}

impl WizardRuntime {
    pub fn start() -> anyhow::Result<Self> {
        let rt = Runtime::new()?;
        let handle = rt.handle().clone();
        // Park the runtime in a dedicated thread so it stays alive for the
        // lifetime of the wizard. `pending::<()>` never resolves, so the
        // worker keeps draining its task queue until the process exits.
        let _thread = thread::Builder::new()
            .name("wizard-runtime".into())
            .spawn(move || {
                rt.block_on(async {
                    std::future::pending::<()>().await;
                });
            })?;
        Ok(Self { handle, _thread })
    }
}
