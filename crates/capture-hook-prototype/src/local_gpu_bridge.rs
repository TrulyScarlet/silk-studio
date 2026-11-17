//! Three-texture local ownership boundary between shared transport and encoder.

use crate::publication_pool::{Publication, PublicationPool};
use crate::{MAX_HEIGHT, MAX_WIDTH, SLOT_COUNT};
use encoder_api::{GpuFrameContext, GpuFrameResolveError, GpuFrameResolver, GpuFrameResource};
use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use media_types::GpuFrameHandle;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct BudgetState {
    active: usize,
    retired: Vec<Weak<GpuTextureSlot>>,
    failed: bool,
}
#[derive(Default)]
pub struct GpuPoolBudget {
    state: Mutex<BudgetState>,
}

pub struct LocalGpuBridge {
    pool: Arc<PublicationPool<GpuTextureSlot>>,
    device: Arc<GpuDeviceContext>,
    dimensions: (u32, u32),
    budget: Arc<GpuPoolBudget>,
    retired: AtomicBool,
}

impl LocalGpuBridge {
    pub fn new(
        device: Arc<GpuDeviceContext>,
        width: u32,
        height: u32,
        budget: Arc<GpuPoolBudget>,
    ) -> Result<Arc<Self>, String> {
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            return Err("invalid bounded local GPU dimensions".into());
        }
        {
            let mut state = budget
                .state
                .lock()
                .map_err(|_| "GPU pool budget poisoned")?;
            if state.failed {
                return Err("GPU pool budget is closed after a registry failure".into());
            }
            state.retired.retain(|surface| surface.strong_count() > 0);
            if state.active + state.retired.len() + SLOT_COUNT > 2 * SLOT_COUNT {
                return Err(
                    "retained GPU frames/contexts exhaust the six-surface restart budget".into(),
                );
            }
            state.active += SLOT_COUNT;
        }
        let surfaces = (|| {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            let mut textures = Vec::with_capacity(SLOT_COUNT);
            for _ in 0..SLOT_COUNT {
                let mut texture = None;
                unsafe {
                    device
                        .device()
                        .CreateTexture2D(&desc, None, Some(&mut texture))
                }
                .map_err(|error| error.to_string())?;
                textures.push(GpuTextureSlot::new(
                    texture.ok_or("null local capture texture")?,
                    device.clone(),
                ));
            }
            textures
                .try_into()
                .map_err(|_| "invalid local capture texture count".to_string())
        })();
        let surfaces = match surfaces {
            Ok(surfaces) => surfaces,
            Err(error) => {
                if let Ok(mut state) = budget.state.lock() {
                    state.active -= SLOT_COUNT;
                }
                return Err(error);
            }
        };
        Ok(Arc::new(Self {
            pool: PublicationPool::new(surfaces),
            device,
            dimensions: (width, height),
            budget,
            retired: AtomicBool::new(false),
        }))
    }
    pub fn context(self: &Arc<Self>) -> GpuFrameContext {
        GpuFrameContext::new(self.clone())
    }
    pub fn copy_imported(
        self: &Arc<Self>,
        source: &GpuTextureSlot,
    ) -> Result<Option<Publication<GpuTextureSlot>>, String> {
        self.copy_imported_after(source, || Ok(()))
    }

    /// Schedule an interop GPU wait only after a destination is reserved, never
    /// when the output pool is full. The callback must not wait on CPU work.
    pub(crate) fn copy_imported_after(
        self: &Arc<Self>,
        source: &GpuTextureSlot,
        before_copy: impl FnOnce() -> Result<(), String>,
    ) -> Result<Option<Publication<GpuTextureSlot>>, String> {
        if self.retired.load(Ordering::Acquire) {
            return Err("GPU bridge is retired".into());
        }
        if source.dimensions() != self.dimensions
            || !Arc::ptr_eq(source.device_context(), &self.device)
        {
            return Err("imported frame changed GPU device or dimensions".into());
        }
        let handle = NEXT_HANDLE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| {
                next.checked_add(1)
            })
            .map_err(|_| "GPU handle sequence exhausted")?;
        let Some(publication) = self.pool.try_publish(handle) else {
            return Ok(None);
        };
        before_copy()?;
        // ImportedFrame::release flushes this same protected immediate context
        // before ownership is returned to the producer. No CPU readback occurs.
        unsafe {
            self.device
                .context()
                .CopyResource(publication.surface().texture(), source.texture());
        }
        Ok(Some(publication))
    }
    pub fn retire(&self) {
        if self.retired.swap(true, Ordering::AcqRel) {
            return;
        }
        self.pool.invalidate();
        let tokens = self.pool.retirement_tokens();
        if let Ok(mut budget) = self.budget.state.lock() {
            budget.active -= SLOT_COUNT;
            budget.retired.retain(|surface| surface.strong_count() > 0);
            if let Some(tokens) = tokens {
                budget.retired.extend(tokens);
            }
            // If the pool mutex was poisoned, prevent any further allocation.
            else {
                budget.failed = true;
            }
        }
    }
}
impl GpuFrameResolver for LocalGpuBridge {
    fn resolve(
        &self,
        handle: GpuFrameHandle,
    ) -> Result<Arc<GpuFrameResource>, GpuFrameResolveError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(GpuFrameResolveError::ContextUnavailable {
                details: "local GPU capture epoch is retired".into(),
            });
        }
        self.pool
            .resolve(handle.0)
            .map(|surface| surface as Arc<GpuFrameResource>)
            .ok_or(GpuFrameResolveError::StaleHandle { handle })
    }
    fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        (!self.retired.load(Ordering::Acquire))
            .then(|| self.device.clone() as Arc<GpuFrameResource>)
    }
}
impl Drop for LocalGpuBridge {
    fn drop(&mut self) {
        self.retire();
    }
}
