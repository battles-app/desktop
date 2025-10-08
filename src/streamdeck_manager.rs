use elgato_streamdeck::{new_hidapi, list_devices, StreamDeck, info::Kind};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxButton {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub is_global: bool, // true for battle board, false for user FX
    pub position: usize, // Original position in the list
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ButtonState {
    pub is_playing: bool,
    pub button: Option<FxButton>,
}

pub struct StreamDeckManager {
    device: Option<StreamDeck>,
    button_states: HashMap<u8, ButtonState>,
    button_layout: Vec<Option<FxButton>>, // Maps Stream Deck button index to FX button
    device_kind: Option<Kind>,
    is_connected: bool,
}

impl StreamDeckManager {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            device: None,
            button_states: HashMap::new(),
            button_layout: Vec::new(),
            device_kind: None,
            is_connected: false,
        })
    }
    
    /// Scan for connected Stream Deck devices
    pub fn scan_devices(&mut self) -> Result<Vec<(Kind, String)>, String> {
        let hid = new_hidapi().map_err(|e| format!("Failed to initialize HidApi: {}", e))?;
        let devices = list_devices(&hid);
        Ok(devices)
    }
    
    /// Connect to the first available Stream Deck device
    pub fn connect(&mut self) -> Result<String, String> {
        let hid = new_hidapi().map_err(|e| format!("Failed to initialize HidApi: {}", e))?;
        let devices = list_devices(&hid);
        
        if devices.is_empty() {
            return Err("No Stream Deck devices found".to_string());
        }
        
        let (kind, serial) = &devices[0];
        
        let device = StreamDeck::connect(&hid, *kind, serial)
            .map_err(|e| format!("Failed to connect to Stream Deck: {}", e))?;
        
        self.device_kind = Some(*kind);
        
        // Get device info
        let serial_number = device.serial_number()
            .unwrap_or_else(|_| "Unknown".to_string());
        
        let info = format!(
            "Connected to {:?} (Serial: {})",
            kind,
            serial_number
        );
        
        // Set initial brightness
        let _ = device.set_brightness(50);
        
        // Clear all buttons
        for i in 0..self.button_count() as u8 {
            let size = self.get_button_size();
            let img = image::RgbaImage::new(size, size);
            let dynamic_img = image::DynamicImage::ImageRgba8(img);
            let _ = device.set_button_image(i, dynamic_img);
        }
        let _ = device.flush();
        
        self.device = Some(device);
        self.is_connected = true;
        
        Ok(info)
    }
    
    /// Disconnect from Stream Deck
    pub fn disconnect(&mut self) {
        if let Some(device) = self.device.take() {
            let _ = device.reset();
        }
        self.is_connected = false;
        self.button_states.clear();
        self.button_layout.clear();
    }
    
    /// Check if device is connected
    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
    
    /// Get device kind name
    pub fn device_kind_name(&self) -> String {
        match self.device_kind {
            Some(kind) => format!("{:?}", kind),
            None => "Unknown".to_string(),
        }
    }
    
    /// Get serial number
    pub fn get_serial_number(&self) -> Result<String, String> {
        if let Some(ref device) = self.device {
            device.serial_number()
                .map_err(|e| format!("Failed to get serial: {}", e))
        } else {
            Err("No device connected".to_string())
        }
    }
    
    /// Get number of buttons on the device
    pub fn button_count(&self) -> usize {
        match self.device_kind {
            Some(Kind::Original) => 15,
            Some(Kind::OriginalV2) => 15,
            Some(Kind::Mk2) => 15,
            Some(Kind::Mk2Scissor) => 15,
            Some(Kind::Mini) => 6,
            Some(Kind::MiniMk2) => 6,
            Some(Kind::Xl) => 32,
            Some(Kind::XlV2) => 32,
            Some(Kind::Plus) => 8,
            Some(Kind::Neo) => 8,
            Some(Kind::Pedal) => 3,
            None => 0,
        }
    }
    
    /// Get button size for the device
    fn get_button_size(&self) -> u32 {
        match self.device_kind {
            Some(Kind::Original) | Some(Kind::OriginalV2) |
            Some(Kind::Mk2) | Some(Kind::Mk2Scissor) | 
            Some(Kind::Mini) | Some(Kind::MiniMk2) => 72,
            Some(Kind::Xl) | Some(Kind::XlV2) => 96,
            Some(Kind::Plus) | Some(Kind::Neo) => 200,
            Some(Kind::Pedal) => 0,
            None => 72,
        }
    }
    
    /// Update button layout with FX buttons
    /// Battle board effects go on left side, user FX on right side
    pub fn update_layout(&mut self, battle_board: Vec<FxButton>, user_fx: Vec<FxButton>) -> Result<(), String> {
        let button_count = self.button_count();
        if button_count == 0 {
            return Err("No device connected".to_string());
        }
        
        // Initialize layout with None
        self.button_layout = vec![None; button_count];
        
        // Get device dimensions for layout calculation
        let (cols, rows) = match self.device_kind {
            Some(Kind::Original) | Some(Kind::OriginalV2) | Some(Kind::Mk2) | Some(Kind::Mk2Scissor) => (5, 3),
            Some(Kind::Mini) | Some(Kind::MiniMk2) => (3, 2),
            Some(Kind::Xl) | Some(Kind::XlV2) => (8, 4),
            Some(Kind::Plus) => (4, 2), // 8 buttons in 4x2 grid
            Some(Kind::Neo) => (4, 2), // 8 buttons
            Some(Kind::Pedal) => (3, 1), // 3 pedals
            None => return Err("Unknown device type".to_string()),
        };
        
        // Calculate split point (left side for battle board, right side for user FX)
        let mid_col = cols / 2;
        
        // Place battle board effects on left side (top to bottom, left to right)
        let mut battle_index = 0;
        for row in 0..rows {
            for col in 0..mid_col {
                if battle_index < battle_board.len() {
                    let button_idx = row * cols + col;
                    if button_idx < button_count {
                        self.button_layout[button_idx] = Some(battle_board[battle_index].clone());
                        battle_index += 1;
                    }
                }
            }
        }
        
        // Place user FX on right side (top to bottom, left to right)
        let mut user_index = 0;
        for row in 0..rows {
            for col in mid_col..cols {
                if user_index < user_fx.len() {
                    let button_idx = row * cols + col;
                    if button_idx < button_count {
                        self.button_layout[button_idx] = Some(user_fx[user_index].clone());
                        user_index += 1;
                    }
                }
            }
        }
        
        // Render all buttons
        self.render_all_buttons()?;
        
        Ok(())
    }
    
    /// Render all buttons based on current layout
    fn render_all_buttons(&mut self) -> Result<(), String> {
        if self.device.is_none() {
            return Ok(());
        }
        
        // Collect all button images first
        let mut button_images: Vec<(u8, Option<image::DynamicImage>)> = Vec::new();
        
        for (idx, button_opt) in self.button_layout.iter().enumerate() {
            let image = if let Some(button) = button_opt {
                let is_playing = self.button_states
                    .get(&(idx as u8))
                    .map(|s| s.is_playing)
                    .unwrap_or(false);
                Some(self.create_button_image(button, is_playing)?)
            } else {
                // Empty button
                let size = self.get_button_size();
                let img = image::RgbaImage::new(size, size);
                Some(image::DynamicImage::ImageRgba8(img))
            };
            button_images.push((idx as u8, image));
        }
        
        // Now set all button images
        if let Some(ref mut device) = self.device {
            for (idx, image_opt) in button_images {
                if let Some(image) = image_opt {
                    device.set_button_image(idx, image)
                        .map_err(|e| format!("Failed to set button image: {}", e))?;
                }
            }
            
            // Flush changes to device
            device.flush().map_err(|e| format!("Failed to flush device: {}", e))?;
        }
        
        Ok(())
    }
    
    /// Create button image with text and styling
    fn create_button_image(&self, fx_button: &FxButton, is_playing: bool) -> Result<image::DynamicImage, String> {
        // Get button size
        let size = self.get_button_size();
        
        // Create a simple colored button image
        // In a production app, you would render text and icons here
        let mut img = image::RgbaImage::new(size, size);
        
        // Set background color based on state
        let color = if is_playing {
            // Green when playing
            image::Rgba([0, 255, 0, 255])
        } else if fx_button.is_global {
            // Purple for battle board
            image::Rgba([128, 0, 255, 255])
        } else {
            // Blue for user FX
            image::Rgba([0, 128, 255, 255])
        };
        
        // Fill with color
        for pixel in img.pixels_mut() {
            *pixel = color;
        }
        
        // TODO: Add text rendering for button name
        // This would require a font rendering library like rusttype
        // For now, just return the colored image
        
        Ok(image::DynamicImage::ImageRgba8(img))
    }
    
    /// Clear all buttons
    pub fn clear_all_buttons(&mut self) -> Result<(), String> {
        if self.device.is_none() {
            return Ok(());
        }
        
        let size = self.get_button_size();
        let button_count = self.button_count();
        
        // Create empty images for all buttons
        let mut button_images: Vec<(u8, image::DynamicImage)> = Vec::new();
        for i in 0..button_count as u8 {
            let img = image::RgbaImage::new(size, size);
            button_images.push((i, image::DynamicImage::ImageRgba8(img)));
        }
        
        // Now set all buttons
        if let Some(ref mut device) = self.device {
            for (idx, image) in button_images {
                device.set_button_image(idx, image)
                    .map_err(|e| format!("Failed to clear button: {}", e))?;
            }
            
            device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
        }
        
        Ok(())
    }
    
    /// Get button state changes (returns list of pressed buttons since last check)
    pub fn get_button_changes(&mut self) -> Result<Vec<(u8, bool)>, String> {
        // Note: The elgato-streamdeck library doesn't have async button reading
        // We'll need to implement a polling mechanism in the frontend
        // For now, return an empty list
        Ok(vec![])
    }
    
    /// Handle button press (toggle play/stop)
    pub fn handle_button_press(&mut self, button_idx: u8) -> Option<(String, bool)> {
        // Get the FX button at this position
        let fx_button = self.button_layout.get(button_idx as usize)?.as_ref()?.clone();
        
        // Toggle playing state
        let entry = self.button_states
            .entry(button_idx)
            .or_insert(ButtonState {
                is_playing: false,
                button: Some(fx_button.clone()),
            });
        
        entry.is_playing = !entry.is_playing;
        let new_state = entry.is_playing;
        let fx_id = fx_button.id.clone();
        
        // Update button visual
        if self.device.is_some() {
            let is_playing = new_state;
            if let Ok(image) = self.create_button_image(&fx_button, is_playing) {
                if let Some(ref mut device) = self.device {
                    let _ = device.set_button_image(button_idx, image);
                    let _ = device.flush();
                }
            }
        }
        
        // Return (fx_id, is_playing)
        Some((fx_id, new_state))
    }
    
    /// Update button state (called when FX stops playing)
    pub fn set_button_state(&mut self, fx_id: &str, is_playing: bool) -> Result<(), String> {
        // Find button with this FX ID and update state
        let mut button_to_update: Option<(u8, FxButton)> = None;
        
        for (idx, button_opt) in self.button_layout.iter().enumerate() {
            if let Some(fx_button) = button_opt {
                if fx_button.id == fx_id {
                    button_to_update = Some((idx as u8, fx_button.clone()));
                    break;
                }
            }
        }
        
        if let Some((idx, fx_button)) = button_to_update {
            // Update state
            self.button_states
                .entry(idx)
                .and_modify(|s| s.is_playing = is_playing)
                .or_insert(ButtonState {
                    is_playing,
                    button: Some(fx_button.clone()),
                });
            
            // Update visual
            if self.device.is_some() {
                if let Ok(image) = self.create_button_image(&fx_button, is_playing) {
                    if let Some(ref mut device) = self.device {
                        device.set_button_image(idx, image)
                            .map_err(|e| format!("Failed to set button image: {}", e))?;
                        device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
                    }
                }
            }
        }
        
        Ok(())
    }
}

// Global Stream Deck manager instance
lazy_static::lazy_static! {
    pub static ref STREAMDECK_MANAGER: Arc<Mutex<Option<StreamDeckManager>>> = Arc::new(Mutex::new(None));
}

