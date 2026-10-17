//! Phase 0 spike: run the iroh spike server inside a real Android app through UniFFI.
//!
//! Questions this answers: does the `.so` load (16 KB aligned), does UniFFI
//! async work with a tokio runtime, and does iroh run with the JNI context
//! installed under `panic = "abort"`.

use std::sync::Arc;

use iroh::Endpoint;

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum SpikeError {
    Failed { reason: String },
}

impl std::fmt::Display for SpikeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Failed { reason } = self;
        f.write_str(reason)
    }
}

impl std::error::Error for SpikeError {}

#[derive(uniffi::Object)]
pub struct Spike {
    endpoint: Endpoint,
}

/// Bind an endpoint and start serving. `port` 0 picks a free port.
#[uniffi::export(async_runtime = "tokio")]
pub async fn start_spike(port: u16, no_gso: bool) -> Result<Arc<Spike>, SpikeError> {
    let port = (port != 0).then_some(port);
    let endpoint = iroh_spike::start_server(port, None, no_gso)
        .await
        .map_err(|e| SpikeError::Failed {
            reason: e.to_string(),
        })?;
    Ok(Arc::new(Spike { endpoint }))
}

#[uniffi::export(async_runtime = "tokio")]
impl Spike {
    pub fn endpoint_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    pub fn addresses(&self) -> Vec<String> {
        self.endpoint
            .addr()
            .ip_addrs()
            .map(|a| a.to_string())
            .collect()
    }

    /// Server events since the last call.
    pub fn events(&self) -> Vec<String> {
        iroh_spike::drain_events()
    }

    /// Gracefully close the endpoint. (Never name a UniFFI method `close`.)
    pub async fn shutdown(&self) {
        self.endpoint.close().await;
    }
}

/// Resident set size of this process in kB, for idle-footprint measurements.
#[uniffi::export]
pub fn resident_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.split_whitespace().next().map(str::to_owned))
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// JNI entry points. The only `unsafe` in this crate.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod android_init {
    use std::ffi::c_void;

    use jni::{
        objects::{JClass, JObject},
        JNIEnv,
    };

    /// Called once from Kotlin with the Application context before any endpoint
    /// is created. iroh's DNS resolver and netdev need it to read system
    /// network configuration; without it iroh aborts under `panic = "abort"`.
    #[no_mangle]
    pub extern "system" fn Java_dev_phonebridge_smoke_NativeInit_initContext(
        env: JNIEnv,
        _class: JClass,
        context: JObject,
    ) {
        let (Ok(vm), Ok(global)) = (env.get_java_vm(), env.new_global_ref(&context)) else {
            return;
        };
        let context_ptr = global.as_obj().as_raw() as *mut c_void;
        // The global ref must outlive the process, as required by ndk-context.
        std::mem::forget(global);
        // SAFETY: `vm` is the process JavaVM and `context_ptr` is a leaked
        // global reference, so both stay valid until the process exits.
        unsafe {
            iroh::dns::install_android_jni_context(vm.get_java_vm_pointer().cast(), context_ptr);
        }
    }
}
