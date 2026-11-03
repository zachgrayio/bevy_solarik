//! Compile the real signal producers and guide/resolve consumers, including DLSS coexistence.
#[path = "support/shader_graph.rs"]
mod shader_graph;

#[test]
fn nrd_signals_and_guides_compose_with_and_without_ray_reconstruction() {
    let mut graph = shader_graph::Graph::load();
    graph.validate("bevy_solarik::realtime::nrd", &[]);
    for rr in [false, true] {
        let mut defs = vec![
            "NRD_SIGNALS".into(),
            "BINDLESS_SURFACE_DETAIL".into(),
            bevy_shader::ShaderDefVal::UInt("WORLD_CACHE_SIZE".into(), 1 << 20),
            bevy_shader::ShaderDefVal::UInt("RASTER_SUBPIXEL_BITS".into(), 8),
        ];
        if rr {
            defs.push("DLSS_RR_GUIDE_BUFFERS".into());
        }
        for module in ["restir_di", "restir_gi", "specular_gi"] {
            graph.validate(&format!("bevy_solarik::realtime::{module}"), &defs);
        }
    }
}
