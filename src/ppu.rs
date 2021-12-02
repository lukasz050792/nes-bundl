// Note: For basic testing purposes, this is scanline-accurate. This should
// later be rewritten with cycle-accurate logic once we're past proof of concept
// and prototype stages.

use mmc::mapper::*;

#[derive(Copy, Clone)]
pub struct SpriteLatch {
    tile_index: u8,    
    bitmap_high: u8,
    bitmap_low: u8,
    attributes: u8,
    x_counter: u8,
    y_pos: u8,
    active: bool,
}

impl SpriteLatch {
    pub fn new() -> SpriteLatch {
        return SpriteLatch {
            tile_index: 0,
            bitmap_high: 0x00,
            bitmap_low: 0x00,
            attributes: 0x00,
            x_counter: 0xFF,
            y_pos: 0x00,
            active: false,
        }
    }

    pub fn shift(&mut self) {
        // If we're active at this point, shift the bitmap registers based on our flip direction
        if self.active {
            if self.attributes & 0b0100_0000 != 0 {
                self.bitmap_high = self.bitmap_high >> 1;
                self.bitmap_low = self.bitmap_low >> 1;
            } else {
                self.bitmap_high = self.bitmap_high << 1;
                self.bitmap_low = self.bitmap_low << 1;
            }
            return;
        }

        if self.x_counter > 0 {
            self.x_counter -= 1;
        }
        if self.x_counter == 0 {
            self.active = true;
        }
    }

    pub fn palette(&self) -> u8 {
        return self.attributes & 0b0000_0011;
    }

    pub fn bg_priority(&self) -> bool {
        return self.attributes & 0b0010_0000 != 0;
    }

    pub fn y_flip(&self) -> bool {
        return self.attributes & 0b1000_0000 != 0;
    }

    pub fn palette_index(&self) -> u8 {
        // Return either the high or low bits of the shifter based on x-flip bit
        if self.attributes & 0b0100_0000 != 0 {
            return 
                ((self.bitmap_high & 0b0000_0001) << 1) | 
                 (self.bitmap_low  & 0b0000_0001);
        } else {
            return 
                ((self.bitmap_high & 0b1000_0000) >> 6) | 
                ((self.bitmap_low  & 0b1000_0000) >> 7);
        }
    }
}

pub struct PpuState {
    // PPU Memory (incl. cart CHR ROM for now)
    pub internal_vram: [u8; 0x1000],
    pub oam: [u8; 0x100],
    pub secondary_oam: [SpriteLatch; 8],
    pub secondary_oam_index: usize,
    pub palette: [u8; 32],

    // Memory Mapped Registers
    // PPU Registers
    pub latch: u8,

    // PPU reads from unconnected mapper space (uncommon, but not impossible)
    pub open_bus: u8,

    pub read_buffer: u8,

    pub control: u8,
    pub mask: u8,
    pub status: u8,
    pub oam_addr: u8,

    pub oam_dma_high: u8,

    // Internal
    pub current_frame: u32,
    pub current_scanline: u16,
    pub current_scanline_cycle: u16,    

    // Framebuffer
    pub screen: [u16; 256 * 240],
    pub sprite_color: [u8; 256],
    pub sprite_index: [u8; 256],
    pub sprite_bg_priority: [bool; 256],
    pub sprite_zero: [bool; 256],

    pub write_toggle: bool,

    // Internal State
    pub current_vram_address: u16,
    pub temporary_vram_address: u16,
    pub fine_x: u8,
    pub tile_shift_low: u16,
    pub tile_shift_high: u16,
    pub tile_low: u8,
    pub tile_high: u8,
    pub tile_index: u8,
    pub palette_shift_low: u8,
    pub palette_shift_high: u8,
    pub palette_latch: u8,
    pub attribute_byte: u8,

    pub sprite_zero_on_scanline: bool,

    // Debug Viewer
    pub recent_reads: Vec<u16>,
    pub recent_writes: Vec<u16>,
}

fn debug_default_palette() -> [u8; 32] {
    // Completely arbitrary color selection here, a real NES's boot palette
    // is somewhat random, determined by analog effects and RAM decay.
    // My own NES produces this ugly cyan on failed loads, so that's what
    // we get here.
    return [
        // BG, cool tones (note that 0c becomes global background color by default)
        0x0c, 0x0c, 0x1c, 0x2c,
        0x01, 0x11, 0x21, 0x31,
        0x02, 0x12, 0x22, 0x32,
        0x03, 0x13, 0x23, 0x33,
        // OBJ, warm tones
        0x05, 0x15, 0x25, 0x35,
        0x06, 0x16, 0x26, 0x36,
        0x07, 0x17, 0x27, 0x37,
        0x08, 0x18, 0x28, 0x38,
    ];
}

impl PpuState {
    pub fn new() -> PpuState {
        return PpuState {
            internal_vram: [0_u8; 0x1000],  // 4k for four-screen mirroring, most games only use upper 2k
            oam: [0_u8; 0x100],
            secondary_oam: [SpriteLatch::new(); 8],
            secondary_oam_index: 0,
            palette: debug_default_palette(),
            current_frame: 0,
            current_scanline: 0,
            current_scanline_cycle: 0,
            screen: [0_u16; 256 * 240],
            sprite_color: [0_u8; 256],
            sprite_index: [0_u8; 256],
            sprite_bg_priority: [false; 256],
            sprite_zero: [false; 256],
    
            control: 0,
            mask: 0,
            status: 0,
            oam_addr: 0,
            oam_dma_high: 0,
            latch: 0,
            open_bus: 0,
            read_buffer: 0,
    
            write_toggle: false,
    
            // Internal State
            current_vram_address: 0,
            temporary_vram_address: 0,
            fine_x: 0,
            tile_shift_low: 0,
            tile_shift_high: 0,
            tile_low: 0,
            tile_high: 0,
            tile_index: 0,
            palette_shift_low: 0,
            palette_shift_high: 0,
            palette_latch: 0,
            attribute_byte: 0,
            sprite_zero_on_scanline: false,

            // Debug
            recent_reads: Vec::new(),
            recent_writes: Vec::new(),
       };
    }

    pub fn read_latched_byte(&mut self, mapper: &mut dyn Mapper, address: u16) -> u8 {
        let masked_address = address & 0x3FFF;
        match masked_address {
            0x3F00 ..= 0x3FFF => {
                // We're going to return palette data from read_byte, but we place data from "underneath" the palette
                // space in the read_buffer. This is intentional, a very odd quirk of PPU reading due to the way
                // palette reads are implemented in hardware.
                self.read_buffer = mapper.read_ppu(masked_address).unwrap_or(self.open_bus);
                return self.read_byte(mapper, address);
            },
            _ => {
                let result = self.read_buffer;
                self.read_buffer = self.read_byte(mapper, address);
                return result;
            }
        }
    }

    pub fn debug_read_byte(&self, mapper: &dyn Mapper, address: u16) -> u8 {
        let masked_address = address & 0x3FFF;
        match masked_address {
            0x0000 ..= 0x3EFF => {
                return match mapper.debug_read_ppu(masked_address) {
                    Some(byte) => byte,
                    None => self.open_bus
                };
            },
            0x3F00 ..= 0x3FFF => {
                let mut palette_address = masked_address & 0x1F;
                // Weird background masking
                if palette_address & 0x13 == 0x10 {
                    palette_address = palette_address - 0x10;
                }
                let mut palette_entry = self.palette[palette_address as usize];
                if self.mask & 0b0000_0001 != 0 {
                    palette_entry &= 0x30;
                }
                return palette_entry;
            },
            _ => return 0
        }
    }

    pub fn read_byte(&mut self, mapper: &mut dyn Mapper, address: u16) -> u8 {
        // process side effects here
        let masked_address = address & 0x3FFF;
        match masked_address {
            0x0000 ..= 0x3EFF => {
                //println!("PPU: Read from 0x{:04X}, dot {} of scanline {}", masked_address, self.current_scanline_cycle, self.current_scanline);
                self.open_bus = match mapper.read_ppu(masked_address) {
                    Some(byte) => byte,
                    None => self.open_bus
                };
                return self.open_bus;
            }
            _ => {return self.debug_read_byte(mapper, address);}
        }

    }

    pub fn access_byte(&mut self, mapper: &mut dyn Mapper, address: u16) {
        // process side effects here
        let masked_address = address & 0x3FFF;
        //println!("PPU: Access from 0x{:04X}, dot {} of scanline {}", masked_address, self.current_scanline_cycle, self.current_scanline);
        mapper.access_ppu(masked_address)
    }

    pub fn write_byte(&mut self, mapper: &mut dyn Mapper, address: u16, data: u8) {
        let masked_address = address & 0x3FFF;
        self.recent_writes.insert(0, masked_address);
        self.recent_writes.truncate(20);
        match masked_address {
            0x0000 ..= 0x3EFF => mapper.write_ppu(masked_address, data),
            0x3F00 ..= 0x3FFF => {
                // palette data is 6-bits, so mask off the upper two:
                let palette_entry = data & 0b0011_1111;
                let mut palette_address = masked_address & 0x1F;
                // Weird background masking
                if palette_address & 0x13 == 0x10 {
                    palette_address = palette_address - 0x10;
                }
                self.palette[palette_address as usize] = palette_entry;
            },
            _ => () // Do nothing!
        }
    }

    fn initialize_secondary_oam(&mut self) {
        for i in 0 .. 8 {
            self.secondary_oam[i].tile_index = 0xFF;
            self.secondary_oam[i].active = false;
        }
        self.secondary_oam_index = 0;
    }

    fn evaluate_sprites(&mut self) {
        let scanline = self.current_scanline as u8;
        let mut sprite_size = 8;
        if (self.control & 0x20) != 0 {
            sprite_size = 16;
        }
        self.sprite_zero_on_scanline = false;

        self.initialize_secondary_oam();

        // Gather first 8 visible sprites (and pay attention if there are more)
        for i in 0 .. 64 {
            let y = self.oam[i * 4 + 0];
            if scanline >= y && scanline < y + sprite_size {
                if self.secondary_oam_index < 8 {
                    // Copy this sprite's data into temporary secondary OAM for this scanline
                    self.secondary_oam[self.secondary_oam_index].y_pos =      self.oam[i * 4 + 0];
                    self.secondary_oam[self.secondary_oam_index].tile_index = self.oam[i * 4 + 1];
                    self.secondary_oam[self.secondary_oam_index].attributes = self.oam[i * 4 + 2];
                    self.secondary_oam[self.secondary_oam_index].x_counter  = self.oam[i * 4 + 3];
                    self.secondary_oam[self.secondary_oam_index].active = false;

                    self.secondary_oam_index += 1;
                    if i == 0 {
                        self.sprite_zero_on_scanline = true;
                    }
                } else {
                    self.status = self.status | 0x20; // bit 5 = sprite overflow this frame
                }
            }
        }
    }

    pub fn rendering_enabled(&self) -> bool {
        return (self.mask & 0b0001_1000) != 0;
    }

    fn shift_bg_registers(&mut self) {
        self.tile_shift_high = self.tile_shift_high << 1;
        self.tile_shift_low = self.tile_shift_low << 1;
        self.palette_shift_high = self.palette_shift_high << 1;
        self.palette_shift_low = self.palette_shift_low << 1;
        // Palette data needs to be reloaded from the current latch
        self.palette_shift_high |= (self.palette_latch & 0b10) >> 1;
        self.palette_shift_low  |=  self.palette_latch & 0b01;
    }

    fn reload_shift_registers(&mut self) {
        self.tile_shift_high &= 0xFF00;
        self.tile_shift_high |= self.tile_high as u16;
        self.tile_shift_low &= 0xFF00;
        self.tile_shift_low |= self.tile_low as u16;
        // Use coarse X and coarse Y (tile indices) to determine which palette to use from
        // the attribute byte, and apply that to the palette latch
        //                                          nn yyyyy xxxxx
        let attr_x = (self.current_vram_address & 0b00_00000_00010) >> 1;
        let attr_y = (self.current_vram_address & 0b00_00010_00000) >> 6;
        let palette_shift = ((attr_y << 1) | attr_x) * 2;
        self.palette_latch = (self.attribute_byte >> palette_shift) & 0b11;
    }

    fn plot_pixel(&mut self, x: u16, y: u16, color: u8) {
        let index = ((y as usize) * 256) + (x as usize);
        let pixel_color = (((self.mask as u16) & 0b1110_0000) << 1) | ((color as u16) & 0b0011_1111);
        self.screen[index] = pixel_color;
    }

    fn draw_pixel(&mut self, mapper: &mut dyn Mapper) {
        // Output a pixel based on the current background shifters
        let bg_x_bit = 0b1000_0000_0000_0000 >> self.fine_x;
        let bg_x_shift = 15 - self.fine_x;
        let mut bg_palette_index = 
            ((self.tile_shift_high & bg_x_bit) >> (bg_x_shift - 1)) | 
            ((self.tile_shift_low & bg_x_bit) >> bg_x_shift);

        let attr_x_bit = 0b1000_0000 >> self.fine_x;
        let attr_x_shift = 7 - self.fine_x;
        let bg_palette_high = (self.palette_shift_high & attr_x_bit) >> attr_x_shift;
        let bg_palette_low  = (self.palette_shift_low  & attr_x_bit) >> attr_x_shift;
        let mut bg_palette_number = (bg_palette_high << 1) | bg_palette_low;

        let px = self.current_scanline_cycle - 1;
        let py = self.current_scanline;

        // If backgrounds are disabled, ignore all that work above, and switch to color 0
        if self.mask & 0b0000_1000 == 0 || ((self.mask & 0b0000_0010 == 0) && px < 8) {
            bg_palette_index = 0;
        }

        if bg_palette_index == 0 {
            // bg color 0 always uses the first palette
            bg_palette_number = 0;
        }

        let mut pixel_color = self.read_byte(mapper, (((bg_palette_number as u16) << 2) + bg_palette_index) as u16 + 0x3F00);

        // If sprites are enabled
        if self.mask & 0b0001_0000 != 0 && ((self.mask & 0b0000_0100 != 0) || px >= 8) {
            // Iterate over sprites in reverse order, and find the lowest numbered sprite with an opaque pixel:
            let mut sprite_index = 8;
            for i in (0 .. self.secondary_oam_index).rev() {
                if self.secondary_oam[i].active && self.secondary_oam[i].palette_index() != 0 {
                    // Mark this as the lowest active sprite
                    sprite_index = i;
                }
            }
            if sprite_index < 8 {
                if self.sprite_zero_on_scanline && sprite_index == 0 && bg_palette_index != 0 {
                    // Sprite zero hit!
                    self.status = self.status | 0x40;
                }
                if bg_palette_index == 0 || !self.secondary_oam[sprite_index].bg_priority() {
                    let sprite_palette_number = self.secondary_oam[sprite_index].palette() as u16;
                    let sprite_palette_index = self.secondary_oam[sprite_index].palette_index() as u16;
                    pixel_color = self.read_byte(mapper, (sprite_palette_number << 2) + sprite_palette_index + 0x3F10);
                }
            }
        }

        self.plot_pixel(px, py, pixel_color);
    }

    pub fn increment_coarse_x(&mut self) {
        let mut coarse_x = self.current_vram_address & 0b00_00000_11111;
        coarse_x += 1;
        // If we overflowed Coarse X then
        if coarse_x > 0b11111 {
            // Switch to the adjacent horizontal nametable
            self.current_vram_address ^= 0b01_00000_00000;
        }
        self.current_vram_address = (self.current_vram_address & 0b111_11_11111_00000) | (coarse_x & 0b11111);
    }

    pub fn increment_coarse_y(&mut self) {
        let mut coarse_y = (self.current_vram_address & 0b00_11111_00000) >> 5;
        coarse_y += 1;
        if coarse_y == 30 {
            coarse_y = 0;
            self.current_vram_address ^= 0b10_00000_00000;
        }
        self.current_vram_address = (self.current_vram_address & 0b111_11_00000_11111) | ((coarse_y & 0b11111) << 5);
    }

    pub fn fine_y(&self) -> u16 {
        return (self.current_vram_address & 0b111_00_00000_00000) >> 12;
    }

    pub fn increment_fine_y(&mut self) {
        let mut fine_y = self.fine_y();
        fine_y += 1;
        if fine_y > 7 {
            fine_y = 0;
            self.increment_coarse_y();
        }
        self.current_vram_address &= 0b000_11_11111_11111;
        self.current_vram_address |= (fine_y & 0b111) << 12;
    }

    fn access_bg_tile_early(&mut self, mapper: &mut dyn Mapper) {
        // "fetch" the first byte of CHR tile 0 early, and throw it away
        // This simulates an oddity with the address bus
        // that primarily affects MMC3 IRQ timings. Anything snooping A0-A13
        // changes needs to see this address show up on cycle 0; a RD / WR is NOT
        // performed, however

        let mut pattern_address: u16 = 0x0000;
        if (self.control & 0x10) != 0 {
            pattern_address = 0x1000;
        }

        let tile_low_address = pattern_address + 
            (self.tile_index as u16 * 16) + 
             self.fine_y();
        self.access_byte(mapper, tile_low_address);
    }

    fn fetch_bg_tile(&mut self, mapper: &mut dyn Mapper, sub_cycle: u16) {
        let mut pattern_address: u16 = 0x0000;
        if (self.control & 0x10) != 0 {
            pattern_address = 0x1000;
        }

        match sub_cycle {
            // Documentation for these addresses: https://wiki.nesdev.com/w/index.php/PPU_scrolling#Tile_and_attribute_fetching
            // Note that mirroring is applied in _read_byte, not here.
            0 => {
                let tile_address = 0x2000 | (self.current_vram_address & 0x0FFF);
                self.tile_index = self.read_byte(mapper, tile_address);
            },
            2 => {
                let attribute_address = 
                    0x23C0 | 
                     (self.current_vram_address & 0x0C00) | 
                    ((self.current_vram_address >> 4) & 0x38) | 
                    ((self.current_vram_address >> 2) & 0x07);
                self.attribute_byte = self.read_byte(mapper, attribute_address);
            },
            4 => {
                let tile_low_address = pattern_address + 
                    (self.tile_index as u16 * 16) + 
                     self.fine_y();
                self.tile_low = self.read_byte(mapper, tile_low_address);
            },
            6 => {
                let tile_high_address = pattern_address + 
                    (self.tile_index as u16 * 16) + 8 +
                     self.fine_y();
                self.tile_high = self.read_byte(mapper, tile_high_address);
            },
            7 => {
                self.reload_shift_registers();
                self.increment_coarse_x();
            },
            _ => ()
        }
    }

    fn fetch_sprite_tiles(&mut self, mapper: &mut dyn Mapper) {
        let sub_cycle = (self.current_scanline_cycle - 257) % 8;
        match sub_cycle {
            // Note: the nametable address fetches here are thrown away, but they are performed, and
            // do affect any mappers listening to PPU activity. I'm unsuire of the address that should be
            // fetched here, but existing documentation seems to suggest it would simply be whatever the
            // vram counter currently points to, without any updates to that counter:
            // http://nesdev.com/2C02%20technical%20reference.TXT
            0  | 2 => {
                let tile_address = 0x2000 | (self.current_vram_address & 0x0FFF);
                let _ = self.read_byte(mapper, tile_address);
            },
            _ => {}
        }
        if sub_cycle == 4 || sub_cycle == 6 {
            let sprite_index: usize = ((self.current_scanline_cycle - 257) / 8) as usize;
            let mut tile_index = self.secondary_oam[sprite_index].tile_index;

            let mut sprite_size: u16 = 8;
            if (self.control & 0b0010_0000) != 0 {
                sprite_size = 16;
            }

            let mut pattern_address: u16 = 0x0000;
            // If we're using 8x16 sprites, set the pattern based on the sprite's tile index
            if sprite_size == 16 {
                if (tile_index & 0b1) != 0 {
                    pattern_address = 0x1000;
                }
                tile_index &= 0b1111_1110;
            } else {
                // Otherwise, the pattern is selected by PPUCTL
                if (self.control & 0b0000_1000) != 0 {
                    pattern_address = 0x1000;
                }
            }

            let mut y_offset = self.current_scanline.wrapping_sub(self.secondary_oam[sprite_index].y_pos as u16);
            if self.secondary_oam[sprite_index].y_flip() {
                y_offset = sprite_size.wrapping_sub(1).wrapping_sub(y_offset);
            }

            if y_offset >= 8 {
                y_offset = y_offset.wrapping_sub(8);
                tile_index = tile_index.wrapping_add(1);
            }
            y_offset = y_offset % 8;

            let tile_address = (((tile_index as u16 * 16) + y_offset) & 0xFFF) | pattern_address;

            match sub_cycle {
                4 => self.secondary_oam[sprite_index].bitmap_low  = self.read_byte(mapper, tile_address),
                6 => self.secondary_oam[sprite_index].bitmap_high = self.read_byte(mapper, tile_address + 8),
                _ => ()
            }
        }
    }

    fn shift_sprites(&mut self) {
        for i in 0 .. self.secondary_oam_index {
            self.secondary_oam[i].shift();
        }
    }

    fn prerender_scanline(&mut self, mapper: &mut dyn Mapper) {
        // Setup for next full frame
        match self.current_scanline_cycle {
            1 => {
                // Clear vblank, sprite overflow and sprite zero hit
                self.status = self.status & 0x1F;
                if self.rendering_enabled() {
                    self.fetch_bg_tile(mapper, 0);
                }
            },
            2 ..= 256 => {
                if self.rendering_enabled() {
                    let sub_cycle = (self.current_scanline_cycle - 1) % 8;
                    self.fetch_bg_tile(mapper, sub_cycle);  
                }
            },
            257 => {
                if self.rendering_enabled() {
                    // Reload the X scroll components
                    self.current_vram_address &= 0b111_10_11111_00000;
                    self.current_vram_address |= self.temporary_vram_address & 0b01_00000_11111;
                    // Initialize the sprite table, so we don't end up drawing garbage
                    // to the main display on the first scanline
                    self.initialize_secondary_oam();
                    self.fetch_sprite_tiles(mapper);
                }
            },
            258 ..= 279 => {
                if self.rendering_enabled() {
                    self.fetch_sprite_tiles(mapper);
                }
            },
            280 ..= 304 => {
                if self.rendering_enabled() {
                    // Reload the Y scroll components
                    self.current_vram_address &= 0b000_01_00000_11111;
                    self.current_vram_address |= self.temporary_vram_address & 0b111_10_11111_00000;
                    self.fetch_sprite_tiles(mapper);
                }
            },
            305 ..= 320 => {
                if self.rendering_enabled() {
                    self.fetch_sprite_tiles(mapper);
                }
            }
            321 ..= 336 => {
                if self.rendering_enabled() {
                    self.shift_bg_registers();
                    // Fetch nametable tiles for the first two tiles on the next scanline
                    let sub_cycle = (self.current_scanline_cycle - 321) % 8;
                    self.fetch_bg_tile(mapper, sub_cycle);
                }
            },
            // Dummy fetch nametable address for next scanline x2
            // (Required for MMU5 to detect scanlines for IRQs, among other things.)
            337 => {
                if self.rendering_enabled() {
                    let tile_address = 0x2000 | (self.current_vram_address & 0x0FFF);
                    self.tile_index = self.read_byte(mapper, tile_address);
                }
            },
            339 => {
                if self.rendering_enabled() {
                    let tile_address = 0x2000 | (self.current_vram_address & 0x0FFF);
                    self.tile_index = self.read_byte(mapper, tile_address);
                }
            },
            340 => {
                if self.rendering_enabled() {
                    if self.current_frame & 0x1 != 0 {
                        // Skip ahead one cycle on odd frames. This jitter produces a cleaner image
                        // for NTSC signal generation.

                        // (note: the effect here is to skip to cycle 1 of scanline 0, since this
                        // counter is immediately incremented)
                        self.current_scanline_cycle = 0;
                        self.current_scanline = 0;
                        self.current_frame += 1;
                    }
                }
            }
            _ => ()
        }
    }

    fn render_scanline(&mut self, mapper: &mut dyn Mapper) {
        if self.rendering_enabled() {
            match self.current_scanline_cycle {
                0 => {
                    //println!("PPU Access: dot {} of scanline {} on frame {}", self.current_scanline_cycle, self.current_scanline, self.current_frame);
                    self.access_bg_tile_early(mapper);
                },
                1 ..= 256 => {
                    self.draw_pixel(mapper);
                    self.shift_bg_registers();
                    self.shift_sprites();
                    let sub_cycle = (self.current_scanline_cycle - 1) % 8;
                    self.fetch_bg_tile(mapper, sub_cycle);
                    
                    if self.current_scanline_cycle == 256 {
                        self.increment_fine_y();
                    }
                },
                257 ..= 320 => {
                    if self.current_scanline_cycle == 257 {
                        // Reload the X scroll components
                        self.current_vram_address &= 0b111_10_11111_00000;
                        self.current_vram_address |= self.temporary_vram_address & 0b01_00000_11111;

                        // Evaluate all the sprites. Technically the real PPU does this during background
                        // rendering, but we do it all at once. As far as I'm aware, this doesn't affect
                        // external state.
                        self.evaluate_sprites();
                    }
                    self.fetch_sprite_tiles(mapper);
                },
                321 ..= 336 => {
                    self.shift_bg_registers();
                    // Fetch nametable tiles for the first two tiles on the next scanline
                    let sub_cycle = (self.current_scanline_cycle - 321) % 8;
                    self.fetch_bg_tile(mapper, sub_cycle);
                },
                // Dummy fetch nametable address for next scanline x2
                // (Required for MMU5 to detect scanlines for IRQs, among other things.)
                337 | 339 => {
                    let tile_address = 0x2000 | (self.current_vram_address & 0x0FFF);
                    self.tile_index = self.read_byte(mapper, tile_address);
                },
                _ => ()
            }
        } else {
            match self.current_scanline_cycle {
                // cycle 0 is a dummy cycle, nothing happens
                1 ..= 256 => {
                    // The PPU is disabled. Usually, we should show the backdrop color:
                    let mut pixel_color = self.read_byte(mapper, 0x3F00);
                    // However, if the current VRAM address is within palette memory, instead
                    // show whatever that color is:
                    if self.current_vram_address >= 0x3F00 && self.current_vram_address <= 0x3FFF {
                        let vram_address = self.current_vram_address;
                        pixel_color = self.read_byte(mapper, vram_address);
                    }

                    let px = self.current_scanline_cycle - 1;
                    let py = self.current_scanline;
                    self.plot_pixel(px, py, pixel_color);
                },
                _ => ()
            }
        }
    }

    fn vblank_scanline(&mut self) {
        if self.current_scanline_cycle == 1 {
            // VBlank! Set NMI flag here
            self.status = (self.status & 0x7F) + 0x80;
        }
    }

    pub fn clock(&mut self, mapper: &mut dyn Mapper) {
        match self.current_scanline {
            0 ..= 239 => self.render_scanline(mapper),
            240 => {
                if self.current_scanline_cycle == 1 && self.rendering_enabled() {
                    // When scanline 240 is reached, rendering ends and the contents of v are immediately placed
                    // on the bus. (They stay there until rendering begins or PPUADDR is changed by the program.)
                    let vram_address = self.current_vram_address;
                    let _ = self.read_byte(mapper, vram_address);
                }
            }
            241 => self.vblank_scanline(),
            261 => self.prerender_scanline(mapper),
            _ => ()
        }

        self.current_scanline_cycle += 1;
        if self.current_scanline_cycle > 340 {
            self.current_scanline_cycle = 0;
            self.current_scanline += 1;
            if self.current_scanline > 261 {
                self.current_scanline = 0;
                self.current_frame += 1;
            }
        }
    }

    pub fn get_bg_tile(&self, mapper: &dyn Mapper, tx: u8, ty: u8) -> u8 {
        let mut address: u16 = 0x2000;
        if tx > 31 {
            address = address + 0x0400;
        }
        if ty > 29 {
            address = address + 0x0800;
        }
        address = address + ((ty % 30) as u16) * 32 + ((tx & 0x1F) as u16);
        return self.debug_read_byte(mapper, address);
    }

    pub fn get_bg_palette(&self, mapper: &dyn Mapper, tx: u8, ty: u8) -> u8 {
        let mut address: u16 = 0x23C0;
        if tx > 31 {
            address = address + 0x0400;
        }
        if ty > 29 {
            address = address + 0x0800;
        }
        address += ((tx & 0x1F) >> 2) as u16;
        address += (((ty % 30) >> 2) as u16)* 0x8;
        let attr_byte = self.debug_read_byte(mapper, address);
        let shift = (((tx & 0x2) >> 1) + ((ty % 30) & 0x2)) << 1;
        let mask = 0x3 << shift;
        return (attr_byte & mask) >> shift;
    }
}

pub fn nametable_address(read_address: u16, mirroring: Mirroring) -> u16 {
    // Mirroring documented here, the ABCD references to the charts in this article:
    // https://wiki.nesdev.com/w/index.php/Mirroring
    let nt_address = read_address & 0x3FF;
    let nt_offset = (0x000, 0x400, 0x800, 0xC00);
    match read_address {
        // Nametable 0 (top-left)
        0x2000 ..= 0x23FF => {
            return match mirroring {
                Mirroring::Horizontal       => nt_address + nt_offset.0, // A
                Mirroring::Vertical         => nt_address + nt_offset.0, // A
                Mirroring::OneScreenLower   => nt_address + nt_offset.0, // A
                Mirroring::OneScreenUpper   => nt_address + nt_offset.1, // B
                Mirroring::FourScreen       => nt_address + nt_offset.0, // A
            }
        },
        // Nametable 1 (top-right)
        0x2400 ..= 0x27FF => {
            return match mirroring {
                Mirroring::Horizontal       => nt_address + nt_offset.0, // A
                Mirroring::Vertical         => nt_address + nt_offset.1, // B
                Mirroring::OneScreenLower   => nt_address + nt_offset.0, // A
                Mirroring::OneScreenUpper   => nt_address + nt_offset.1, // B
                Mirroring::FourScreen       => nt_address + nt_offset.1, // B
            }
        },
        // Nametable 2 (bottom-left)
        0x2800 ..= 0x2BFF => {
            return match mirroring {
                Mirroring::Horizontal       => nt_address + nt_offset.1, // B
                Mirroring::Vertical         => nt_address + nt_offset.0, // A
                Mirroring::OneScreenLower   => nt_address + nt_offset.0, // A
                Mirroring::OneScreenUpper   => nt_address + nt_offset.1, // B
                Mirroring::FourScreen       => nt_address + nt_offset.2, // C
            }
        },
        // Nametable 3 (bottom-right)
        0x2C00 ..= 0x2FFF => {
            return match mirroring {
                Mirroring::Horizontal       => nt_address + nt_offset.1, // B
                Mirroring::Vertical         => nt_address + nt_offset.1, // B
                Mirroring::OneScreenLower   => nt_address + nt_offset.0, // A
                Mirroring::OneScreenUpper   => nt_address + nt_offset.1, // B
                Mirroring::FourScreen       => nt_address + nt_offset.3, // D
            }
        },
        _ => return 0, // wat
    }
}
