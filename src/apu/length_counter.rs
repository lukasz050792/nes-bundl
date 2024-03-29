use crate::save_load::*;

pub struct LengthCounterState {
    pub length: u8,
    pub halt_flag: bool,
    pub channel_enabled: bool,
}

impl LengthCounterState{
    pub fn new() -> LengthCounterState {
        return LengthCounterState {
            length: 0,
            halt_flag: false,
            channel_enabled: false,
        }
    }

    pub fn clock(&mut self) {
        if self.channel_enabled {
            if self.length > 0 && !(self.halt_flag) {
                self.length -= 1;
            }
        } else {
            self.length = 0;
        }
    }

    pub fn set_length(&mut self, index: u8) {
        if self.channel_enabled {
            let table = [
                10, 254, 20,  2, 40,  4, 80,  6, 160,  8, 60, 10, 14, 12, 26, 14,
                12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30];
            self.length = table[index as usize];
        } else {
            self.length = 0
        }
    }

    pub fn save_state(&self, buff: &mut Vec<u8>) {
        save_u8(buff, self.length);
        save_bool(buff, self.halt_flag);
        save_bool(buff, self.channel_enabled);
    }

    pub fn load_state(&mut self, buff: &mut Vec<u8>) {
        load_bool(buff, &mut self.channel_enabled);
        load_bool(buff, &mut self.halt_flag);
        load_u8(buff, &mut self.length);
    }
}