use drm::{
    ClientCapability, Device,
    control::{
        self, Device as ControlDevice, ModeTypeFlags, PlaneType,
        connector::{self, State},
        plane,
    },
};
use raw_window_handle::{DrmDisplayHandle, DrmWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io,
    os::{
        fd::AsRawFd,
        unix::io::{AsFd, BorrowedFd},
    },
};
use wgpu::SurfaceTargetUnsafe;

#[derive(Debug)]
pub struct Drm {
    card: Card,
    window: DrmWindow,
}

impl Drm {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_card(Card::open_default()?)
    }

    fn with_card(card: Card) -> Result<Self, Box<dyn Error>> {
        let Some(drm_window) = card.drm_window()? else {
            return Err("Could not initialize DRM".into());
        };
        Ok(Self {
            card,
            window: drm_window,
        })
    }

    pub fn mode(&self) -> &control::Mode {
        &self.window.mode
    }

    pub fn window_surface_target(&self) -> SurfaceTargetUnsafe {
        SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Drm(DrmDisplayHandle::new(
                self.card.as_fd().as_raw_fd(),
            ))),
            raw_window_handle: RawWindowHandle::Drm(DrmWindowHandle::new(
                self.window.plane_handle.into(),
            )),
        }
    }

    pub fn drm_surface_target(&self) -> SurfaceTargetUnsafe {
        let mode = self.mode();
        let refresh_rate = (((mode.clock() as f64 * 1000.0)
            / (mode.hsync().2 as f64 * mode.vsync().2 as f64))
            * 1000.0)
            .round() as u32;
        let (width, height) = mode.size();
        SurfaceTargetUnsafe::Drm {
            fd: self.card.as_fd().as_raw_fd(),
            plane: self.window.plane_handle.into(),
            connector_id: self.window.connector_handle.into(),
            width: width as u32,
            height: height as u32,
            refresh_rate,
        }
    }
}

#[derive(Debug)]
struct Card(File);

#[derive(Debug)]
struct DrmWindow {
    mode: control::Mode,
    connector_handle: connector::Handle,
    plane_handle: plane::Handle,
}

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

impl Card {
    fn open(path: &str) -> Result<Self, io::Error> {
        let this = Self(OpenOptions::new().read(true).write(true).open(path)?);
        this.set_client_capability(ClientCapability::UniversalPlanes, true)?;
        Ok(this)
    }

    fn open_default() -> Result<Self, io::Error> {
        Self::open("/dev/dri/card0")
    }

    fn drm_window(&self) -> Result<Option<DrmWindow>, io::Error> {
        let Some((connector_handle, plane_handle, mode)) = self.initialize()? else {
            return Ok(None);
        };
        Ok(Some(DrmWindow {
            mode,
            connector_handle,
            plane_handle,
        }))
    }

    fn initialize(
        &self,
    ) -> Result<Option<(connector::Handle, plane::Handle, control::Mode)>, io::Error> {
        let resources = self.resource_handles()?;

        for &connector_handle in resources.connectors() {
            let Ok(connector_info) = self.get_connector(connector_handle, true) else {
                continue;
            };
            if connector_info.state() != State::Connected {
                continue;
            }
            let modes = connector_info.modes();
            let preferred_mode = modes
                .iter()
                .find(|&mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                .or_else(|| modes.first());

            let Some(crtc_handle) = connector_info
                .encoders()
                .iter()
                .find_map(|&encoder_handle| {
                    resources
                        .filter_crtcs(self.get_encoder(encoder_handle).ok()?.possible_crtcs())
                        .into_iter()
                        .next()
                })
            else {
                continue;
            };

            let current_mode = self.get_crtc(crtc_handle)?.mode();
            if current_mode.as_ref() != preferred_mode {
                eprintln!(
                    "Using current mode {current_mode:?} not preferred mode {preferred_mode:?}"
                );
                // XXX we should modeset https://github.com/Smithay/drm-rs/blob/develop/examples/atomic_modeset.rs
            }
            let Some(mode) = current_mode else {
                continue;
            };

            for plane_handle in self.plane_handles()? {
                if !matches!(self.plane_type(plane_handle)?, Some(PlaneType::Primary)) {
                    continue;
                }
                let plane_info = self.get_plane(plane_handle)?;
                if resources
                    .filter_crtcs(plane_info.possible_crtcs())
                    .contains(&crtc_handle)
                {
                    return Ok(Some((connector_handle, plane_handle, mode)));
                }
            }
        }
        Ok(None)
    }

    fn plane_type(&self, plane: control::plane::Handle) -> io::Result<Option<PlaneType>> {
        let props = self.get_properties(plane)?;

        for (&prop_handle, &raw_value) in props.iter() {
            let info = self.get_property(prop_handle)?;
            if info.name().to_bytes() == b"type" {
                let ty = match raw_value as u32 {
                    x if x == PlaneType::Primary as u32 => Some(PlaneType::Primary),
                    x if x == PlaneType::Overlay as u32 => Some(PlaneType::Overlay),
                    x if x == PlaneType::Cursor as u32 => Some(PlaneType::Cursor),
                    _ => None,
                };
                return Ok(ty);
            }
        }

        Ok(None)
    }
}
