// The resolved GPU router: desugars the two settings.json variants (simple
// `render_node`(+`scanout_node`) / advanced `gpu_router`) into ONE model every
// downstream reader consumes. No logging dep — sits beside config.base.
pub mod router;
