//! Input device enumeration.

use cpal::traits::{DeviceTrait, HostTrait};

/// One selectable microphone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// Stable identifier (`host:device` form, cpal `DeviceId`).
    pub id: String,
    /// Human-readable name shown in settings.
    pub name: String,
    pub is_default: bool,
}

/// Lists input devices of the default host. Devices that fail to
/// report an id or description are skipped.
#[must_use]
pub fn list_input_devices() -> Vec<InputDeviceInfo> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok());
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let id = device.id().ok()?;
            let description = device.description().ok()?;
            Some(InputDeviceInfo {
                id: id.to_string(),
                name: description.name().to_owned(),
                is_default: Some(&id) == default_id.as_ref(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::list_input_devices;

    /// Hardware-dependent smoke test; run manually with
    /// `cargo test -p pw-audio -- --ignored`.
    #[test]
    #[ignore = "requires audio hardware"]
    fn enumerates_at_least_one_input_device() {
        let devices = list_input_devices();
        assert!(!devices.is_empty());
        assert!(devices.iter().any(|device| device.is_default));
    }
}
