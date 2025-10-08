// Stream Deck Integration Example for Battles.app
// This file shows how to integrate Stream Deck functionality into the Vue.js frontend

import { invoke } from '@tauri-apps/api/tauri';
import { listen } from '@tauri-apps/api/event';

// Type definitions
interface FxButton {
  id: string;
  name: string;
  image_url?: string;
  is_global: boolean;
  position: number;
}

interface StreamDeckInfo {
  connected: boolean;
  device_name: string;
  button_count: number;
  serial_number: string | null;
}

interface StreamDeckButtonPressEvent {
  button_idx: number;
  fx_id: string;
  should_play: boolean;
}

// Stream Deck Manager Class
export class StreamDeckManager {
  private isInitialized = false;
  private isConnected = false;
  private unlistenConnected: (() => void) | null = null;
  private unlistenDisconnected: (() => void) | null = null;
  private unlistenButtonPress: (() => void) | null = null;

  // Initialize Stream Deck system
  async initialize(): Promise<void> {
    try {
      console.log('[Stream Deck] Initializing...');
      await invoke('streamdeck_init');
      this.isInitialized = true;
      
      // Try to connect
      await this.connect();
      
      // Setup event listeners
      await this.setupEventListeners();
      
      console.log('[Stream Deck] ✅ Initialized and ready');
    } catch (error) {
      console.error('[Stream Deck] ❌ Initialization failed:', error);
      throw error;
    }
  }

  // Connect to Stream Deck
  async connect(): Promise<string> {
    try {
      const result = await invoke<string>('streamdeck_connect');
      this.isConnected = true;
      console.log('[Stream Deck] ✅ Connected:', result);
      return result;
    } catch (error) {
      console.error('[Stream Deck] ❌ Connection failed:', error);
      this.isConnected = false;
      throw error;
    }
  }

  // Disconnect from Stream Deck
  async disconnect(): Promise<void> {
    try {
      await invoke('streamdeck_disconnect');
      this.isConnected = false;
      console.log('[Stream Deck] Disconnected');
    } catch (error) {
      console.error('[Stream Deck] Disconnect error:', error);
    }
  }

  // Get device info
  async getInfo(): Promise<StreamDeckInfo> {
    return await invoke<StreamDeckInfo>('streamdeck_get_info');
  }

  // Scan for devices
  async scan(): Promise<string[]> {
    return await invoke<string[]>('streamdeck_scan');
  }

  // Update button layout
  async updateLayout(battleBoard: FxButton[], userFx: FxButton[]): Promise<void> {
    try {
      console.log('[Stream Deck] Updating layout:', {
        battleBoard: battleBoard.length,
        userFx: userFx.length
      });
      
      await invoke('streamdeck_update_layout', {
        battleBoard,
        userFx
      });
      
      console.log('[Stream Deck] ✅ Layout updated');
    } catch (error) {
      console.error('[Stream Deck] ❌ Layout update failed:', error);
      throw error;
    }
  }

  // Set button playing state
  async setButtonState(fxId: string, isPlaying: boolean): Promise<void> {
    try {
      await invoke('streamdeck_set_button_state', {
        fxId,
        isPlaying
      });
    } catch (error) {
      console.error('[Stream Deck] Set button state error:', error);
    }
  }

  // Setup event listeners
  private async setupEventListeners(): Promise<void> {
    // Listen for device connection
    this.unlistenConnected = await listen('streamdeck://connected', () => {
      console.log('[Stream Deck] 🔌 Device connected');
      this.isConnected = true;
      this.onDeviceConnected();
    });

    // Listen for device disconnection
    this.unlistenDisconnected = await listen('streamdeck://disconnected', () => {
      console.log('[Stream Deck] 🔌 Device disconnected');
      this.isConnected = false;
      this.onDeviceDisconnected();
    });

    // Listen for button presses
    this.unlistenButtonPress = await listen<StreamDeckButtonPressEvent>(
      'streamdeck://button-press',
      (event) => {
        const { button_idx, fx_id, should_play } = event.payload;
        console.log('[Stream Deck] 🎮 Button pressed:', {
          button: button_idx,
          fx: fx_id,
          play: should_play
        });
        this.onButtonPress(fx_id, should_play);
      }
    );
  }

  // Cleanup event listeners
  cleanup(): void {
    if (this.unlistenConnected) this.unlistenConnected();
    if (this.unlistenDisconnected) this.unlistenDisconnected();
    if (this.unlistenButtonPress) this.unlistenButtonPress();
  }

  // Override these methods in your implementation
  protected onDeviceConnected(): void {
    // Called when device connects
    // You should reload the layout here
  }

  protected onDeviceDisconnected(): void {
    // Called when device disconnects
  }

  protected onButtonPress(fxId: string, shouldPlay: boolean): void {
    // Called when a button is pressed
    // You should play/stop the FX here
  }
}

// Example usage in a Vue component
export function useStreamDeck() {
  const streamDeck = ref<StreamDeckManager | null>(null);

  // Create custom manager with handlers
  class CustomStreamDeckManager extends StreamDeckManager {
    protected onDeviceConnected(): void {
      // Update layout when device connects
      this.refreshLayout();
    }

    protected onDeviceDisconnected(): void {
      // Show notification
      console.warn('Stream Deck disconnected! Waiting for reconnection...');
    }

    protected onButtonPress(fxId: string, shouldPlay: boolean): void {
      if (shouldPlay) {
        // Play the FX
        this.playFx(fxId);
      } else {
        // Stop the FX
        this.stopFx(fxId);
      }
    }

    private async refreshLayout(): Promise<void> {
      // Get current FX from your state management
      const battleBoard = this.getBattleBoardFx();
      const userFx = this.getUserFx();
      
      await this.updateLayout(battleBoard, userFx);
    }

    private getBattleBoardFx(): FxButton[] {
      // Get from your global FX state
      return globalFxItems.value.map((item: any) => ({
        id: item.id,
        name: item.name,
        image_url: item.imageUrl,
        is_global: true,
        position: item.position || 0
      }));
    }

    private getUserFx(): FxButton[] {
      // Get from your user FX state
      return fxFiles.value
        .map((file: any, index: number) => {
          if (!file) return null;
          
          return {
            id: `fxfile${(index + 1).toString().padStart(3, '0')}`,
            name: fxNames.value[index] || `FX ${index + 1}`,
            image_url: file.id ? `/directus-assets/${file.id}` : undefined,
            is_global: false,
            position: index
          };
        })
        .filter((item: any) => item !== null) as FxButton[];
    }

    private async playFx(fxId: string): Promise<void> {
      // Call your existing FX play function
      if (fxId.startsWith('fxfile')) {
        // User FX
        const index = parseInt(fxId.replace('fxfile', '')) - 1;
        await playFxFile(index);
      } else {
        // Global FX
        await playGlobalFx(fxId);
      }
    }

    private async stopFx(fxId: string): Promise<void> {
      // Call your existing FX stop function
      if (fxId.startsWith('fxfile')) {
        // User FX
        const index = parseInt(fxId.replace('fxfile', '')) - 1;
        await stopFxFile(index);
      } else {
        // Global FX
        await stopGlobalFx(fxId);
      }
    }
  }

  // Initialize on mount
  onMounted(async () => {
    try {
      streamDeck.value = new CustomStreamDeckManager();
      await streamDeck.value.initialize();
    } catch (error) {
      console.error('Failed to initialize Stream Deck:', error);
      // Stream Deck is optional, don't block app startup
    }
  });

  // Cleanup on unmount
  onUnmounted(() => {
    if (streamDeck.value) {
      streamDeck.value.cleanup();
    }
  });

  // Watch for FX changes and update layout
  watch([globalFxItems, fxFiles], async () => {
    if (streamDeck.value) {
      const battleBoard = streamDeck.value.getBattleBoardFx();
      const userFx = streamDeck.value.getUserFx();
      await streamDeck.value.updateLayout(battleBoard, userFx);
    }
  }, { deep: true });

  // Sync button state when FX ends
  const onFxEnded = async (fxId: string) => {
    if (streamDeck.value) {
      await streamDeck.value.setButtonState(fxId, false);
    }
  };

  return {
    streamDeck,
    onFxEnded
  };
}

// Example integration in DashboardView.vue
/*
<script setup lang="ts">
import { useStreamDeck } from '~/composables/useStreamDeck';

const { streamDeck, onFxEnded } = useStreamDeck();

// Call onFxEnded when FX finishes playing
const handleFxVideoEnd = async () => {
  showFxPreview.value = false;
  currentFxVideo.value = null;
  currentFxChromaKey.value = false;
  
  // Notify Stream Deck that FX ended
  if (currentFxId.value) {
    await onFxEnded(currentFxId.value);
  }
};
</script>
*/

