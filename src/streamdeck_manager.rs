use elgato_streamdeck::{new_hidapi, StreamDeck, StreamDeckKind, ButtonStateUpdate};
use std::sync::Arc;
use parking_lot::RwLock;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

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
    hid: Arc<hidapi::HidApi>,
    button_states: HashMap<u8, ButtonState>,
    button_layout: Vec<Option<FxButton>>, // Maps Stream Deck button index to FX button
    device_kind: Option<StreamDeckKind>,
    is_connected: bool,
}

impl StreamDeckManager {
    pub fn new() -> Result<Self, String> {
        let hid = new_hidapi().map_err(|e| format!("Failed to initialize HidApi: {}", e))?;
        
        Ok(Self {
            device: None,
            hid: Arc::new(hid),
            button_states: HashMap::new(),
            button_layout: Vec::new(),
            device_kind: None,
            is_connected: false,
        })
    }
    
    /// Scan for connected Stream Deck devices
    pub fn scan_devices(&mut self) -> Result<Vec<(StreamDeckKind, String)>, String> {
        let devices = StreamDeck::list_devices(&self.hid);
        Ok(devices)
    }
    
    /// Connect to the first available Stream Deck device
    pub fn connect(&mut self) -> Result<String, String> {
        let devices = self.scan_devices()?;
        
        if devices.is_empty() {
            return Err("No Stream Deck devices found".to_string());
        }
        
        let (kind, serial) = &devices[0];
        
        let device = StreamDeck::connect(&self.hid, *kind, serial)
            .map_err(|e| format!("Failed to connect to Stream Deck: {}", e))?;
        
        self.device_kind = Some(*kind);
        self.device = Some(device);
        self.is_connected = true;
        
        // Get device info
        let info = format!(
            "Connected to {} (Serial: {})",
            self.device_kind_name(),
            self.get_serial_number().unwrap_or_else(|_| "Unknown".to_string())
        );
        
        // Set initial brightness
        if let Some(ref mut dev) = self.device {
            let _ = dev.set_brightness(50);
        }
        
        // Clear all buttons
        self.clear_all_buttons()?;
        
        Ok(info)
    }
    
    /// Disconnect from Stream Deck
    pub fn disconnect(&mut self) {
        if let Some(mut device) = self.device.take() {
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
            Some(StreamDeckKind::Original) => 15,
            Some(StreamDeckKind::OriginalV2) => 15,
            Some(StreamDeckKind::Mk2) => 15,
            Some(StreamDeckKind::Mini) => 6,
            Some(StreamDeckKind::MiniMk2) => 6,
            Some(StreamDeckKind::Xl) => 32,
            Some(StreamDeckKind::XlV2) => 32,
            Some(StreamDeckKind::Plus) => 8,
            Some(StreamDeckKind::Neo) => 8,
            Some(StreamDeckKind::Pedal) => 3,
            None => 0,
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
            Some(StreamDeckKind::Original) | Some(StreamDeckKind::OriginalV2) | Some(StreamDeckKind::Mk2) => (5, 3),
            Some(StreamDeckKind::Mini) | Some(StreamDeckKind::MiniMk2) => (3, 2),
            Some(StreamDeckKind::Xl) | Some(StreamDeckKind::XlV2) => (8, 4),
            Some(StreamDeckKind::Plus) => (4, 2), // 8 buttons in 4x2 grid
            Some(StreamDeckKind::Neo) => (4, 2), // 8 buttons
            Some(StreamDeckKind::Pedal) => (3, 1), // 3 pedals
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
        if let Some(ref mut device) = self.device {
            for (idx, button_opt) in self.button_layout.iter().enumerate() {
                if let Some(button) = button_opt {
                    self.render_button(device, idx as u8, button)?;
                } else {
                    // Clear empty button
                    self.clear_button(device, idx as u8)?;
                }
            }
            
            // Flush changes to device
            if device.updated {
                device.flush().map_err(|e| format!("Failed to flush device: {}", e))?;
            }
        }
        
        Ok(())
    }
    
    /// Render a single button with text and optional image
    fn render_button(&mut self, device: &mut StreamDeck, button_idx: u8, fx_button: &FxButton) -> Result<(), String> {
        // Get button state
        let is_playing = self.button_states
            .get(&button_idx)
            .map(|s| s.is_playing)
            .unwrap_or(false);
        
        // Create button image with text
        let image = self.create_button_image(fx_button, is_playing)?;
        
        // Set button image
        device.set_button_image(button_idx, image)
            .map_err(|e| format!("Failed to set button image: {}", e))?;
        
        Ok(())
    }
    
    /// Create button image with text and styling
    fn create_button_image(&self, fx_button: &FxButton, is_playing: bool) -> Result<image::DynamicImage, String> {
        // Get button size based on device type
        let size = match self.device_kind {
            Some(StreamDeckKind::Original) | Some(StreamDeckKind::OriginalV2) => 72,
            Some(StreamDeckKind::Mk2) | Some(StreamDeckKind::Mini) | Some(StreamDeckKind::MiniMk2) => 72,
            Some(StreamDeckKind::Xl) | Some(StreamDeckKind::XlV2) => 96,
            Some(StreamDeckKind::Plus) | Some(StreamDeckKind::Neo) => 200, // Touch screen buttons
            Some(StreamDeckKind::Pedal) => 0, // No display
            None => 72,
        };
        
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
    
    /// Clear a single button
    fn clear_button(&mut self, device: &mut StreamDeck, button_idx: u8) -> Result<(), String> {
        let size = match self.device_kind {
            Some(StreamDeckKind::Original) | Some(StreamDeckKind::OriginalV2) |
            Some(StreamDeckKind::Mk2) | Some(StreamDeckKind::Mini) | Some(StreamDeckKind::MiniMk2) => 72,
            Some(StreamDeckKind::Xl) | Some(StreamDeckKind::XlV2) => 96,
            Some(StreamDeckKind::Plus) | Some(StreamDeckKind::Neo) => 200,
            _ => 72,
        };
        
        let img = image::RgbaImage::new(size, size);
        let dynamic_img = image::DynamicImage::ImageRgba8(img);
        
        device.set_button_image(button_idx, dynamic_img)
            .map_err(|e| format!("Failed to clear button: {}", e))?;
        
        Ok(())
    }
    
    /// Clear all buttons
    pub fn clear_all_buttons(&mut self) -> Result<(), String> {
        if let Some(ref mut device) = self.device {
            for i in 0..self.button_count() as u8 {
                self.clear_button(device, i)?;
            }
            
            if device.updated {
                device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
            }
        }
        
        Ok(())
    }
    
    /// Read button states (non-blocking)
    pub fn read_buttons(&mut self, callback: impl Fn(u8, bool)) -> Result<(), String> {
        if let Some(ref mut device) = self.device {
            // Read button states
            while let Ok(updates) = device.read_buttons(Some(std::time::Duration::from_millis(0))) {
                for update in updates {
                    match update {
                        ButtonStateUpdate::ButtonDown(idx) => {
                            callback(idx, true);
                        }
                        ButtonStateUpdate::ButtonUp(idx) => {
                            callback(idx, false);
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle button press (toggle play/stop)
    pub fn handle_button_press(&mut self, button_idx: u8) -> Option<(String, bool)> {
        // Get the FX button at this position
        if let Some(Some(fx_button)) = self.button_layout.get(button_idx as usize) {
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
            if let Some(ref mut device) = self.device {
                let _ = self.render_button(device, button_idx, fx_button);
                if device.updated {
                    let _ = device.flush();
                }
            }
            
            // Return (fx_id, is_playing)
            return Some((fx_id, new_state));
        }
        
        None
    }
    
    /// Update button state (called when FX stops playing)
    pub fn set_button_state(&mut self, fx_id: &str, is_playing: bool) -> Result<(), String> {
        // Find button with this FX ID
        for (idx, button_opt) in self.button_layout.iter().enumerate() {
            if let Some(fx_button) = button_opt {
                if fx_button.id == fx_id {
                    // Update state
                    self.button_states
                        .entry(idx as u8)
                        .and_modify(|s| s.is_playing = is_playing)
                        .or_insert(ButtonState {
                            is_playing,
                            button: Some(fx_button.clone()),
                        });
                    
                    // Update visual
                    if let Some(ref mut device) = self.device {
                        self.render_button(device, idx as u8, fx_button)?;
                        if device.updated {
                            device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
                        }
                    }
                    
                    return Ok(());
                }
            }
        }
        
        Ok(())
    }
}

// Global Stream Deck manager instance
lazy_static::lazy_static! {
    pub static ref STREAMDECK_MANAGER: Arc<RwLock<Option<StreamDeckManager>>> = Arc::new(RwLock::new(None));
}

