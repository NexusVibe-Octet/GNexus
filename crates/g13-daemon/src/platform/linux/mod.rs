//! Linux (reference target: CachyOS / Arch, recent kernels, Wayland + X11).
//!
//! These modules compile today and define the exact shape of the Linux
//! back-ends; their bodies are filled in during the Linux port. Required
//! permissions are provided by `deploy/linux/70-gnexus-uaccess.rules`.

pub mod access;
pub mod audio;
pub mod eq;
pub mod foreground;
pub mod gamepad;
pub mod injector;
pub mod media;
pub mod recorder;
