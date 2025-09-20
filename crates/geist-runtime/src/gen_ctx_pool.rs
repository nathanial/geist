use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::{Receiver, Sender, bounded};
use geist_world::{GenCtx, HeightTileStats, World};

/// Lock-free pool for reusing costly `GenCtx` instances across worker jobs.
pub struct GenCtxPool {
    available_tx: Sender<GenCtx>,
    available_rx: Receiver<GenCtx>,
    allocated: AtomicUsize,
    max_contexts: usize,
}

impl GenCtxPool {
    pub fn new(max_contexts: usize) -> Self {
        debug_assert!(max_contexts > 0);
        let (tx, rx) = bounded(max_contexts);
        Self {
            available_tx: tx,
            available_rx: rx,
            allocated: AtomicUsize::new(0),
            max_contexts,
        }
    }

    /// Acquire a context from the pool, creating a new one if under capacity.
    pub fn acquire<'pool>(&'pool self, world: &World) -> PooledGenCtx<'pool> {
        if let Ok(mut ctx) = self.available_rx.try_recv() {
            Self::prepare(&mut ctx);
            return PooledGenCtx {
                ctx: Some(ctx),
                pool: self,
            };
        }

        loop {
            let current = self.allocated.load(Ordering::Acquire);
            if current < self.max_contexts {
                let prev = self.allocated.fetch_add(1, Ordering::AcqRel);
                if prev < self.max_contexts {
                    let mut ctx = world.make_gen_ctx();
                    Self::prepare(&mut ctx);
                    return PooledGenCtx {
                        ctx: Some(ctx),
                        pool: self,
                    };
                }
                self.allocated.fetch_sub(1, Ordering::AcqRel);
            }

            match self.available_rx.recv() {
                Ok(mut ctx) => {
                    Self::prepare(&mut ctx);
                    return PooledGenCtx {
                        ctx: Some(ctx),
                        pool: self,
                    };
                }
                Err(_) => continue,
            }
        }
    }

    fn prepare(ctx: &mut GenCtx) {
        ctx.terrain_profiler.reset();
        ctx.height_tile_stats = HeightTileStats::default();
    }

    fn release(&self, ctx: GenCtx) {
        let _ = self.available_tx.send(ctx);
    }
}

pub struct PooledGenCtx<'pool> {
    ctx: Option<GenCtx>,
    pool: &'pool GenCtxPool,
}

impl<'pool> Deref for PooledGenCtx<'pool> {
    type Target = GenCtx;

    fn deref(&self) -> &Self::Target {
        self.ctx.as_ref().expect("GenCtx already released")
    }
}

impl<'pool> DerefMut for PooledGenCtx<'pool> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx.as_mut().expect("GenCtx already released")
    }
}

impl<'pool> Drop for PooledGenCtx<'pool> {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.take() {
            self.pool.release(ctx);
        }
    }
}

impl GenCtxPool {
    pub fn with_capacity_from_workers(worker_count: usize) -> Arc<Self> {
        let count = worker_count.max(1) * 2;
        Arc::new(Self::new(count))
    }
}
