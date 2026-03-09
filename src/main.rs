pub fn main() {
    let drm = wgpu_drm::drm::Drm::new().expect("DRM should be initialized");
    let (width, height) = drm.mode().size();
    pollster::block_on(wgpu_drm::render::run(
        // drm.drm_surface_target(),
        drm.window_surface_target(),
        width as u32,
        height as u32,
    ));
}
