use elgato_streamdeck::{new_hidapi, list_devices};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDeckDiagnostics {
    pub hidapi_initialized: bool,
    pub devices_found: usize,
    pub device_details: Vec<DeviceDetail>,
    pub driver_status: DriverStatus,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDetail {
    pub kind: String,
    pub serial: String,
    pub vendor_id: u16,
    pub product_id: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DriverStatus {
    Ok,
    NeedsDrivers,
    Unknown,
}

pub fn run_diagnostics() -> StreamDeckDiagnostics {
    let mut diagnostics = StreamDeckDiagnostics {
        hidapi_initialized: false,
        devices_found: 0,
        device_details: Vec::new(),
        driver_status: DriverStatus::Unknown,
        recommendations: Vec::new(),
    };

    // Try to initialize HidApi
    match new_hidapi() {
        Ok(hid) => {
            diagnostics.hidapi_initialized = true;
            
            // List all HID devices (not just Stream Decks) for debugging
            let _all_devices: Vec<_> = hid.device_list().collect();
            
            // List Stream Deck devices
            let devices = list_devices(&hid);
            diagnostics.devices_found = devices.len();
            
            for (kind, serial) in devices {
                let (vendor_id, product_id) = (kind.vendor_id(), kind.product_id());
                diagnostics.device_details.push(DeviceDetail {
                    kind: format!("{:?}", kind),
                    serial: serial.clone(),
                    vendor_id,
                    product_id,
                });
            }
            
            // Check driver status
            if diagnostics.devices_found > 0 {
                diagnostics.driver_status = DriverStatus::Ok;
                diagnostics.recommendations.push(
                    format!("✅ Found {} Stream Deck device(s). Ready to connect!", diagnostics.devices_found)
                );
            } else {
                diagnostics.driver_status = DriverStatus::NeedsDrivers;
                
                // Check if any Elgato devices are visible at all
                let elgato_devices: Vec<_> = hid.device_list()
                    .filter(|d| d.vendor_id() == 0x0fd9) // Elgato vendor ID
                    .collect();
                
                if elgato_devices.is_empty() {
                    diagnostics.recommendations.push(
                        "❌ No Stream Deck devices detected.".to_string()
                    );
                    diagnostics.recommendations.push(
                        "Please check:".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  1. Is your Stream Deck plugged in via USB?".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  2. Try a different USB cable".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  3. Try a different USB port (preferably USB 3.0)".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  4. Restart your computer".to_string()
                    );
                    
                    #[cfg(target_os = "windows")]
                    {
                        diagnostics.recommendations.push(
                            "  5. Install the official Elgato Stream Deck software (includes drivers):".to_string()
                        );
                        diagnostics.recommendations.push(
                            "     https://www.elgato.com/downloads".to_string()
                        );
                    }
                } else {
                    diagnostics.recommendations.push(
                        format!("⚠️ Found {} Elgato device(s) but not recognized as Stream Deck.", elgato_devices.len())
                    );
                    diagnostics.recommendations.push(
                        "This might be a driver issue. Try:".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  1. Install the official Elgato Stream Deck software".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  2. Update Windows (for latest HID drivers)".to_string()
                    );
                    diagnostics.recommendations.push(
                        "  3. Restart your computer after installing drivers".to_string()
                    );
                }
            }
        }
        Err(e) => {
            diagnostics.hidapi_initialized = false;
            diagnostics.driver_status = DriverStatus::NeedsDrivers;
            diagnostics.recommendations.push(
                format!("❌ Failed to initialize HID API: {}", e)
            );
            diagnostics.recommendations.push(
                "This usually means system HID drivers are not working properly.".to_string()
            );
            
            #[cfg(target_os = "windows")]
            {
                diagnostics.recommendations.push(
                    "For Windows:".to_string()
                );
                diagnostics.recommendations.push(
                    "  1. Run Windows Update to get latest drivers".to_string()
                );
                diagnostics.recommendations.push(
                    "  2. Install official Elgato Stream Deck software from:".to_string()
                );
                diagnostics.recommendations.push(
                    "     https://www.elgato.com/downloads".to_string()
                );
                diagnostics.recommendations.push(
                    "  3. Restart your computer".to_string()
                );
            }
        }
    }

    diagnostics
}

pub fn get_driver_download_info() -> DriverDownloadInfo {
    DriverDownloadInfo {
        windows: DriverInfo {
            name: "Elgato Stream Deck Software".to_string(),
            url: "https://www.elgato.com/downloads".to_string(),
            notes: "Download and install the Stream Deck software. The installer includes all necessary drivers.".to_string(),
            automatic: false,
        },
        macos: DriverInfo {
            name: "Elgato Stream Deck Software".to_string(),
            url: "https://www.elgato.com/downloads".to_string(),
            notes: "Download and install the Stream Deck software for macOS.".to_string(),
            automatic: false,
        },
        linux: DriverInfo {
            name: "udev rules".to_string(),
            url: "".to_string(),
            notes: "Linux uses udev rules instead of drivers. See STREAMDECK_INTEGRATION.md for setup instructions.".to_string(),
            automatic: false,
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverDownloadInfo {
    pub windows: DriverInfo,
    pub macos: DriverInfo,
    pub linux: DriverInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverInfo {
    pub name: String,
    pub url: String,
    pub notes: String,
    pub automatic: bool,
}

