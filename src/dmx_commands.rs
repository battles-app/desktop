use crate::dmx_manager::{DMX_MANAGER, DmxDevice, DmxState};
use tauri::command;
use serde::{Serialize, Deserialize};

/// Mode data for generic DMX packet building
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModeData {
    pub name: String,
    #[serde(rename = "isModeBased")]
    pub is_mode_based: Option<bool>,
    #[serde(rename = "modeChannelIndex")]
    pub mode_channel_index: Option<usize>,
    #[serde(rename = "modeValueForDMX")]
    pub mode_value_for_dmx: Option<u8>,
    #[serde(rename = "dimmerChannelIndex")]
    pub dimmer_channel_index: Option<usize>,
    #[serde(rename = "strobeChannelIndex")]
    pub strobe_channel_index: Option<usize>,
    #[serde(rename = "rgbStartChannelIndex")]
    pub rgb_start_channel_index: Option<usize>,
    #[serde(rename = "rgbSegmentCount")]
    pub rgb_segment_count: Option<usize>,
    #[serde(rename = "panChannelIndex")]
    pub pan_channel_index: Option<usize>,
    #[serde(rename = "tiltChannelIndex")]
    pub tilt_channel_index: Option<usize>,
    #[serde(rename = "panFineChannelIndex")]
    pub pan_fine_channel_index: Option<usize>,
    #[serde(rename = "tiltFineChannelIndex")]
    pub tilt_fine_channel_index: Option<usize>,
    #[serde(rename = "whiteChannelIndex")]
    pub white_channel_index: Option<usize>,
    #[serde(rename = "warmWhiteChannelIndex")]
    pub warm_white_channel_index: Option<usize>,
    #[serde(rename = "coldWhiteChannelIndex")]
    pub cold_white_channel_index: Option<usize>,
    #[serde(rename = "amberChannelIndex")]
    pub amber_channel_index: Option<usize>,
    #[serde(rename = "panTiltSpeedChannelIndex")]
    pub pan_tilt_speed_channel_index: Option<usize>,
    #[serde(rename = "effectsChannelIndex")]
    pub effects_channel_index: Option<usize>,
    #[serde(rename = "resetChannelIndex")]
    pub reset_channel_index: Option<usize>,
    #[serde(rename = "invertDimmer")]
    pub invert_dimmer: Option<bool>,
    pub channels: Vec<Channel>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Channel {
    pub number: usize,
}

/// Generic DMX packet builder - works for ANY fixture!
fn build_dmx_packet_from_mode(
    mode: &ModeData,
    r: u8,
    g: u8,
    b: u8,
    intensity: u8
) -> Vec<u8> {
    let channel_count = mode.channels.len();
    let mut packet = vec![0; channel_count];
    
    // Set mode selector if needed
    if mode.is_mode_based.unwrap_or(false) {
        if let (Some(idx), Some(val)) = (mode.mode_channel_index, mode.mode_value_for_dmx) {
            if idx < packet.len() {
                packet[idx] = val;
                // println!("[DMX Builder] Mode channel {} = {}", idx, val);
            }
        }
    }
    
    // Determine if we have a dedicated dimmer channel
    let has_dimmer = mode.dimmer_channel_index.is_some();
    
    // Set dimmer if present
    if let Some(idx) = mode.dimmer_channel_index {
        if idx < packet.len() {
            packet[idx] = intensity;
            // println!("[DMX Builder] Dimmer channel {} = {}", idx, intensity);
        }
    }
    
    // Set strobe to 0 (no strobe) if present
    if let Some(idx) = mode.strobe_channel_index {
        if idx < packet.len() {
            packet[idx] = 0;
            // println!("[DMX Builder] Strobe channel {} = 0", idx);
        }
    }
    
    // Set RGB for all segments
    let rgb_start = mode.rgb_start_channel_index.unwrap_or(0);
    let segment_count = mode.rgb_segment_count.unwrap_or(1);
    
    // Calculate final RGB values
    // If no dimmer channel exists, apply intensity by scaling RGB values
    let (final_r, final_g, final_b) = if has_dimmer {
        // Dimmer channel controls intensity, send RGB as-is
        (r, g, b)
    } else {
        // No dimmer channel, scale RGB by intensity (0-255)
        let scale = intensity as f32 / 255.0;
        (
            (r as f32 * scale) as u8,
            (g as f32 * scale) as u8,
            (b as f32 * scale) as u8
        )
    };
    
    // println!("[DMX Builder] RGB: {} segments starting at channel {} (has_dimmer: {}, intensity: {})", 
    //     segment_count, rgb_start, has_dimmer, intensity);
    
    for segment in 0..segment_count {
        let base_idx = rgb_start + (segment * 3);
        if base_idx + 2 < packet.len() {
            packet[base_idx] = final_r;
            packet[base_idx + 1] = final_g;
            packet[base_idx + 2] = final_b;
            // println!("[DMX Builder] Segment {} → R:{} G:{} B:{} (original R:{} G:{} B:{})", 
            //     segment, final_r, final_g, final_b, r, g, b);
        }
    }
    
    packet
}

/// Scan for available DMX devices
#[command]
pub async fn scan_dmx_devices() -> Result<Vec<DmxDevice>, String> {
    DMX_MANAGER.scan_devices()
}

/// Connect to a specific DMX device
#[command]
pub async fn connect_dmx_device(device_id: String) -> Result<(), String> {
    DMX_MANAGER.connect_device(&device_id)
}

/// Disconnect current DMX device
#[command]
pub async fn disconnect_dmx_device() -> Result<(), String> {
    DMX_MANAGER.disconnect()
}

/// Get current DMX state
#[command]
pub async fn get_dmx_state() -> Result<DmxState, String> {
    Ok(DMX_MANAGER.get_state())
}

/// Send DMX data to lights
#[command]
pub async fn send_dmx_data(
    universe: u8,
    start_channel: u16,
    data: Vec<u8>
) -> Result<(), String> {
    DMX_MANAGER.send_dmx(universe, start_channel, &data)
}

/// UNIVERSAL LIGHTING CONTROL - Works for ALL fixtures!
/// Handles RGB, intensity, and optionally pan/tilt for moving heads
#[command]
pub async fn set_dmx_rgb(
    universe: u8,
    start_channel: u16,
    r: u8,
    g: u8,
    b: u8,
    intensity: Option<u8>,
    mode_data: Option<ModeData>,
    // Optional pan/tilt for moving heads (degrees)
    pan: Option<u16>,
    tilt: Option<u16>,
    // Legacy parameters for backward compatibility (not used)
    #[allow(unused_variables)]
    fixture_type: Option<String>,
    #[allow(unused_variables)]
    selected_mode: Option<String>
) -> Result<(), String> {
    let intensity_val = intensity.unwrap_or(255);
    
    // Build DMX packet using generic builder (works for ALL fixtures!)
    let data = if let Some(mode) = mode_data {
        // println!("[DMX Universal] Mode: {} ({} channels)", mode.name, mode.channels.len());
        // if pan.is_some() || tilt.is_some() {
        //     println!("[DMX Universal] Moving head: Pan={:?}°, Tilt={:?}°", pan, tilt);
        // }
        
        let channel_count = mode.channels.len();
        let mut packet = vec![0; channel_count];
        
        // 1. Set mode selector if needed
        if mode.is_mode_based.unwrap_or(false) {
            if let (Some(idx), Some(val)) = (mode.mode_channel_index, mode.mode_value_for_dmx) {
                if idx < packet.len() {
                    packet[idx] = val;
                    // println!("[DMX Universal] Mode CH{} = {}", idx + 1, val);
                }
            }
        }
        
        // 2. Set pan/tilt if provided (moving heads only)
        if let Some(pan_deg) = pan {
            let pan_dmx = ((pan_deg as f32 / 540.0) * 255.0) as u8;
            if let Some(pan_idx) = mode.pan_channel_index {
                if pan_idx < packet.len() {
                    packet[pan_idx] = pan_dmx;
                    // println!("[DMX Universal] Pan CH{} = {} ({}°)", pan_idx + 1, pan_dmx, pan_deg);
                }
            }
        }
        
        if let Some(tilt_deg) = tilt {
            // Use full DMX range (0-255) for tilt to get full movement
            let tilt_dmx = ((tilt_deg as f32 / 270.0) * 255.0) as u8;
            if let Some(tilt_idx) = mode.tilt_channel_index {
                if tilt_idx < packet.len() {
                    packet[tilt_idx] = tilt_dmx;
                    // println!("[DMX Universal] Tilt CH{} = {} ({}°, full range)", tilt_idx + 1, tilt_dmx, tilt_deg);
                }
            }
        }
        
        // 3. Determine if we have a dedicated dimmer channel
        let has_dimmer = mode.dimmer_channel_index.is_some();
        
        // 4. Set dimmer if present
        if let Some(idx) = mode.dimmer_channel_index {
            if idx < packet.len() {
                // Check if dimmer needs to be inverted (some fixtures use 0=full, 255=off)
                let dimmer_value = if mode.invert_dimmer.unwrap_or(false) {
                    255 - intensity_val
                } else {
                    intensity_val
                };
                packet[idx] = dimmer_value;
                // println!("[DMX Universal] Dimmer CH{} = {} (inverted={})", 
                //     idx + 1, dimmer_value, mode.invert_dimmer.unwrap_or(false));
            }
        }
        
        // 5. Set strobe to 0 (no strobe)
        if let Some(idx) = mode.strobe_channel_index {
            if idx < packet.len() {
                packet[idx] = 0;
            }
        }
        
        // Note: All other channels (programs, effects, etc.) are already 0 from packet initialization
        
        // 6. Set RGB for all segments
        let rgb_start = mode.rgb_start_channel_index.unwrap_or(0);
        let segment_count = mode.rgb_segment_count.unwrap_or(1);
        
        // Calculate final RGB values based on dimmer availability
        let (final_r, final_g, final_b) = if has_dimmer {
            (r, g, b)
        } else {
            let scale = intensity_val as f32 / 255.0;
            (
                (r as f32 * scale) as u8,
                (g as f32 * scale) as u8,
                (b as f32 * scale) as u8
            )
        };
        
        // println!("[DMX Universal] RGB: {} segments at CH{} (dimmer={}, intensity={}, has_dimmer={})", 
        //     segment_count, rgb_start + 1, has_dimmer, intensity_val, has_dimmer);
        // println!("[DMX Universal] RGB Input: R:{} G:{} B:{}", r, g, b);
        // println!("[DMX Universal] RGB Final: R:{} G:{} B:{}", final_r, final_g, final_b);
        
        for segment in 0..segment_count {
            let base_idx = rgb_start + (segment * 3);
            if base_idx + 2 < packet.len() {
                packet[base_idx] = final_r;
                packet[base_idx + 1] = final_g;
                packet[base_idx + 2] = final_b;
                // println!("[DMX Universal] Segment {} → packet[{}]={} packet[{}]={} packet[{}]={} (CH{}-{} → R:{} G:{} B:{})", 
                //     segment, 
                //     base_idx, final_r, 
                //     base_idx + 1, final_g, 
                //     base_idx + 2, final_b,
                //     base_idx + 1, base_idx + 3, 
                //     final_r, final_g, final_b);
            } else {
                // println!("[DMX Universal] ⚠️ Segment {} SKIPPED: base_idx={}, packet.len()={}", segment, base_idx, packet.len());
            }
        }
        
        // 7. Set white channel if present (for RGBW fixtures)
        if let Some(white_idx) = mode.white_channel_index {
            if white_idx < packet.len() {
                // Detect if user wants white: R=G=B and all are high (> 200)
                let wants_white = r > 200 && g > 200 && b > 200 && r == g && g == b;
                
                if wants_white {
                    // User selected white color - activate white LEDs
                    let white_value = if has_dimmer {
                        // Let dimmer control intensity
                        255
                    } else {
                        // Apply intensity to white
                        intensity_val
                    };
                    packet[white_idx] = white_value;
                    // println!("[DMX Universal] White CH{} = {} (White mode)", white_idx + 1, white_value);
                    
                    // Turn off RGB when using white
                    for segment in 0..segment_count {
                        let base_idx = rgb_start + (segment * 3);
                        if base_idx + 2 < packet.len() {
                            packet[base_idx] = 0;
                            packet[base_idx + 1] = 0;
                            packet[base_idx + 2] = 0;
                        }
                    }
                } else {
                    // Using RGB colors - turn off white
                    packet[white_idx] = 0;
                    // println!("[DMX Universal] White CH{} = 0 (RGB mode)", white_idx + 1);
                }
            }
        }
        
        // 8. Set warm white channel if present (for WW/CW fixtures)
        if let Some(ww_idx) = mode.warm_white_channel_index {
            if ww_idx < packet.len() {
                // For warm/cold white fixtures, we use the RGB color picker to blend
                // More red/orange = more warm white, more blue = more cold white
                let warm_component = (r as u16 + g as u16) / 2; // Average of R and G for warm
                let warm_value = ((warm_component * intensity_val as u16) / 255) as u8;
                packet[ww_idx] = warm_value;
                // println!("[DMX Universal] Warm White CH{} = {} (R:{} G:{})", ww_idx + 1, warm_value, r, g);
            }
        }
        
        // 9. Set cold white channel if present (for WW/CW fixtures)
        if let Some(cw_idx) = mode.cold_white_channel_index {
            if cw_idx < packet.len() {
                // More blue = more cold white
                let cold_component = b;
                let cold_value = ((cold_component as u16 * intensity_val as u16) / 255) as u8;
                packet[cw_idx] = cold_value;
                // println!("[DMX Universal] Cold White CH{} = {} (B:{})", cw_idx + 1, cold_value, b);
            }
        }
        
        // 10. Set amber channel if present (for CW/WW/A fixtures)
        if let Some(amber_idx) = mode.amber_channel_index {
            if amber_idx < packet.len() {
                // Amber is between red and yellow, so we use red and a bit of green
                let amber_component = if r > 200 && g > 100 && g < 200 {
                    // Detect amber/orange colors
                    ((r as u16 + (g / 2) as u16) / 2) as u8
                } else {
                    0
                };
                let amber_value = ((amber_component as u16 * intensity_val as u16) / 255) as u8;
                packet[amber_idx] = amber_value;
                // println!("[DMX Universal] Amber CH{} = {} (R:{} G:{})", amber_idx + 1, amber_value, r, g);
            }
        }
        
        packet
    } else {
        // Fallback for simple RGB fixtures (3 channels)
        // println!("[DMX Universal] Fallback: simple 3-channel RGB");
        vec![r, g, b]
    };
    
    // println!("[DMX Universal] ✅ Sending {} bytes to {}:{}", data.len(), universe, start_channel);
    // println!("[DMX Universal] 📦 Packet: {:?}", data);
    
    DMX_MANAGER.send_dmx(universe, start_channel, &data)
}

/// Set intensity/dimmer for a light
#[command]
pub async fn set_dmx_dimmer(
    universe: u8,
    channel: u16,
    value: u8
) -> Result<(), String> {
    let data = vec![value];
    DMX_MANAGER.send_dmx(universe, channel, &data)
}

/// Blackout all lights (set all channels to 0)
#[command]
pub async fn dmx_blackout(universe: u8) -> Result<(), String> {
    let data = vec![0; 512];
    DMX_MANAGER.send_dmx(universe, 1, &data)
}

/// Set pan/tilt for moving head lights - GENERIC DATA-DRIVEN APPROACH!
/// Works for ANY moving head fixture by using mode_data from Directus
#[command]
pub async fn set_dmx_pan_tilt(
    universe: u8,
    start_channel: u16,
    pan: u16,  // 0-540 degrees
    tilt: u16, // 0-270 degrees
    mode_data: Option<ModeData>
) -> Result<(), String> {
    // Convert pan/tilt degrees to DMX values (0-255 for 8-bit, or 0-65535 for 16-bit)
    // Most fixtures use 8-bit for basic control
    let pan_dmx = ((pan as f32 / 540.0) * 255.0) as u8;
    let tilt_dmx = ((tilt as f32 / 270.0) * 255.0) as u8;
    
    // println!("[DMX Pan/Tilt] Pan {}° → DMX {}, Tilt {}° → DMX {}", 
    //          pan, pan_dmx, tilt, tilt_dmx);
    
    // Build DMX packet
    let data = if let Some(mode) = mode_data {
        let channel_count = mode.channels.len();
        let mut packet = vec![0; channel_count];
        
        // Set mode selector if needed
        if mode.is_mode_based.unwrap_or(false) {
            if let (Some(idx), Some(val)) = (mode.mode_channel_index, mode.mode_value_for_dmx) {
                if idx < packet.len() {
                    packet[idx] = val;
                }
            }
        }
        
        // Set pan channel
        if let Some(pan_idx) = mode.pan_channel_index {
            if pan_idx < packet.len() {
                packet[pan_idx] = pan_dmx;
                // println!("[DMX Pan/Tilt] Pan channel {} = {}", pan_idx, pan_dmx);
            }
        }
        
        // Set tilt channel
        if let Some(tilt_idx) = mode.tilt_channel_index {
            if tilt_idx < packet.len() {
                packet[tilt_idx] = tilt_dmx;
                // println!("[DMX Pan/Tilt] Tilt channel {} = {}", tilt_idx, tilt_dmx);
            }
        }
        
        packet
    } else {
        // Fallback: assume pan at channel 0, tilt at channel 1
        // println!("[DMX Pan/Tilt] Using fallback: pan ch 0, tilt ch 1");
        vec![pan_dmx, tilt_dmx]
    };
    
    // println!("[DMX Pan/Tilt] Sending {} bytes to {}:{}", data.len(), universe, start_channel);
    DMX_MANAGER.send_dmx(universe, start_channel, &data)
}

/// Set COMPLETE fixture state (RGB + intensity + pan/tilt) - for moving heads
/// This avoids the problem of partial updates zeroing other channels
#[command]
pub async fn set_dmx_complete(
    universe: u8,
    start_channel: u16,
    r: u8,
    g: u8,
    b: u8,
    intensity: u8,
    pan: Option<u16>,  // 0-540 degrees
    tilt: Option<u16>, // 0-270 degrees
    mode_data: Option<ModeData>
) -> Result<(), String> {
    // println!("[DMX Complete] Setting full fixture state: RGB({},{},{}) @ {}, Pan={:?}, Tilt={:?}", 
    //          r, g, b, intensity, pan, tilt);
    
    let data = if let Some(mode) = mode_data {
        let channel_count = mode.channels.len();
        let mut packet = vec![0; channel_count];
        
        // Set mode selector if needed
        if mode.is_mode_based.unwrap_or(false) {
            if let (Some(idx), Some(val)) = (mode.mode_channel_index, mode.mode_value_for_dmx) {
                if idx < packet.len() {
                    packet[idx] = val;
                }
            }
        }
        
        // Set pan/tilt if provided
        if let Some(pan_deg) = pan {
            let pan_dmx = ((pan_deg as f32 / 540.0) * 255.0) as u8;
            if let Some(pan_idx) = mode.pan_channel_index {
                if pan_idx < packet.len() {
                    packet[pan_idx] = pan_dmx;
                    // println!("[DMX Complete] Pan channel {} = {} ({}°)", pan_idx, pan_dmx, pan_deg);
                }
            }
        }
        
        if let Some(tilt_deg) = tilt {
            let tilt_dmx = ((tilt_deg as f32 / 270.0) * 255.0) as u8;
            if let Some(tilt_idx) = mode.tilt_channel_index {
                if tilt_idx < packet.len() {
                    packet[tilt_idx] = tilt_dmx;
                    // println!("[DMX Complete] Tilt channel {} = {} ({}°)", tilt_idx, tilt_dmx, tilt_deg);
                }
            }
        }
        
        // Determine if we have a dedicated dimmer channel
        let has_dimmer = mode.dimmer_channel_index.is_some();
        
        // Set dimmer if present
        if let Some(idx) = mode.dimmer_channel_index {
            if idx < packet.len() {
                packet[idx] = intensity;
                // println!("[DMX Complete] Dimmer channel {} = {}", idx, intensity);
            }
        }
        
        // Set strobe to 0 (no strobe) if present
        if let Some(idx) = mode.strobe_channel_index {
            if idx < packet.len() {
                packet[idx] = 0;
            }
        }
        
        // Set RGB
        let rgb_start = mode.rgb_start_channel_index.unwrap_or(0);
        let segment_count = mode.rgb_segment_count.unwrap_or(1);
        
        // Calculate final RGB values based on dimmer availability
        let (final_r, final_g, final_b) = if has_dimmer {
            (r, g, b)
        } else {
            let scale = intensity as f32 / 255.0;
            (
                (r as f32 * scale) as u8,
                (g as f32 * scale) as u8,
                (b as f32 * scale) as u8
            )
        };
        
        // println!("[DMX Complete] RGB: {} segments starting at channel {}", segment_count, rgb_start);
        
        for segment in 0..segment_count {
            let base_idx = rgb_start + (segment * 3);
            if base_idx + 2 < packet.len() {
                packet[base_idx] = final_r;
                packet[base_idx + 1] = final_g;
                packet[base_idx + 2] = final_b;
                // println!("[DMX Complete] Segment {} → R:{} G:{} B:{}", segment, final_r, final_g, final_b);
            }
        }
        
        packet
    } else {
        // Fallback for simple RGB fixtures
        vec![r, g, b]
    };
    
    // println!("[DMX Complete] Sending {} bytes to {}:{}", data.len(), universe, start_channel);
    DMX_MANAGER.send_dmx(universe, start_channel, &data)
}

/// Handle timeline automation frame (called every frame during playback)
#[command]
pub async fn dmx_automation_frame(
    _light_id: String,
    _r: u8,
    _g: u8,
    _b: u8,
    _intensity: u8
) -> Result<(), String> {
    // This is a lightweight call that just forwards to set_dmx_rgb
    // The timeline component will call this 60 times per second during playback
    
    // TODO: We need to look up the light's DMX address, universe, and mode_data
    // For now, this is a placeholder that shows the architecture
    
    // println!("[DMX Automation] Frame for {}: RGB({},{},{}) @ {}", 
    //     light_id, r, g, b, intensity);
    
    Ok(())
}


