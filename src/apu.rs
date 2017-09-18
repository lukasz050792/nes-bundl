// Note: For basic testing purposes, this is scanline-accurate. This should
// later be rewritten with cycle-accurate logic once we're past proof of concept
// and prototype stages.

use std::fs::OpenOptions;
use std::io::prelude::*;

pub struct PulseChannelState {
    pub enabled: bool,

    // Volume Envelope
    pub volume: u8,
    pub decay: u8,
    pub envelope_enabled: bool,
    pub envelope_loop: bool,
    pub length_enabled: bool,
    pub envelope_start: bool,

    // Frequency Sweep
    pub sweep_enabled: bool,
    pub sweep_period: u8,
    pub sweep_divider: u8,
    pub sweep_negate: bool,
    pub sweep_shift: u8,
    pub sweep_reload: bool,
    // Variance between Pulse 1 and Pulse 2 causes negation to work slightly differently
    pub sweep_ones_compliment: bool,

    pub duty: u8,
    pub sequence_counter: u8,
    pub period_initial: u16,
    pub period_current: u16,
    pub length: u8,
}

impl PulseChannelState {
    pub fn new(sweep_ones_compliment: bool) -> PulseChannelState {
        return PulseChannelState {
            enabled: false,

            // Volume Envelope
            volume: 0,
            decay: 0,
            envelope_enabled: false,
            envelope_loop: false,
            length_enabled: false,
            envelope_start: false,

            // Frequency Sweep
            sweep_enabled: false,
            sweep_period: 0,
            sweep_divider: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_reload: false,
            // Variance between Pulse 1 and Pulse 2 causes negation to work slightly differently
            sweep_ones_compliment: sweep_ones_compliment,

            duty: 0b0000_0001,
            sequence_counter: 0,
            period_initial: 0,
            period_current: 0,
            length: 0,
        }
    }

    pub fn clock(&mut self) {
        if self.period_current == 0 {
            // Reset the period timer, and clock the waveform generator
            self.period_current = self.period_initial;

            // The sequence counter starts at zero, but counts downwards, resulting in an odd
            // lookup sequence of 0, 7, 6, 5, 4, 3, 2, 1
            if self.sequence_counter == 0 {
                self.sequence_counter = 7;
            } else {
                self.sequence_counter -= 1;
            }
        } else {
            self.period_current -= 1;
        }
    }

    pub fn output(&self) -> i16 {
        let mut sample = (self.duty >> self.sequence_counter) & 0b1;
        sample *= self.volume;
        return sample as i16;
    }

    pub fn target_period(&mut self) -> u16 {
        let mut change_amount = self.period_initial >> self.sweep_shift;
        if self.sweep_negate {
            if self.sweep_ones_compliment {
                return self.period_initial - change_amount - 1;
            } else {
                return self.period_initial - change_amount;
            }
        } else {
            return self.period_initial + change_amount;
        }
    }

    pub fn update_sweep(&mut self) {
        let target_period = self.target_period();
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift != 0
        && target_period <= 0x7FF && self.period_initial >= 8 {
            self.period_initial = target_period;
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }
}

pub struct ApuState {
    pub current_cycle: u64,

    pub frame_sequencer_mode: u8,
    pub frame_sequencer: u16,
    pub frame_reset_delay: u8,

    pub frame_interrupt: bool,
    pub disable_interrupt: bool,

    pub pulse_1: PulseChannelState,
    pub pulse_2: PulseChannelState,

    pub sample_buffer: [i16; 4096],
    pub sample_rate: u64,
    pub cpu_clock_rate: u64,
    pub buffer_index: usize,
    pub generated_samples: u64,
    pub next_sample_at: u64,
}

impl ApuState {
    pub fn new() -> ApuState {

        return ApuState {
            current_cycle: 0,
            frame_sequencer_mode: 0,
            frame_sequencer: 0,
            frame_reset_delay: 0,
            frame_interrupt: false,
            disable_interrupt: false,
            pulse_1: PulseChannelState::new(true),
            pulse_2: PulseChannelState::new(false),
            sample_buffer: [0i16; 4096],
            sample_rate: 44100,
            cpu_clock_rate: 1_786_860,
            buffer_index: 0,
            generated_samples: 0,
            next_sample_at: 0,
        }
    }

    pub fn write_register(&mut self, address: u16, data: u8) {
        let duty_table = [
            0b1000_0000,
            0b1100_0000,
            0b1111_0000,
            0b0011_1111,
        ];
        match address {
            0x4000 => {
                let duty_index =      (data & 0b1100_0000) >> 6;
                let length_disable =  (data & 0b0010_0000) != 0;
                let constant_volume = (data & 0b0001_0000) != 0;

                self.pulse_1.duty = duty_table[duty_index as usize];
                self.pulse_1.length_enabled = !(length_disable);
                self.pulse_1.envelope_enabled = !(constant_volume);
                if constant_volume {
                    self.pulse_1.volume = data & 0b0000_1111;
                }
            },
            0x4001 => {
                self.pulse_1.sweep_enabled =  (data & 0b1000_0000) != 0;
                self.pulse_1.sweep_period =   (data & 0b0111_0000) >> 4;
                self.pulse_1.sweep_negate =   (data & 0b0000_1000) != 0;
                self.pulse_1.sweep_shift =     data & 0b0000_0111;
                self.pulse_1.sweep_reload = true;
            },
            0x4002 => {
                let period_low = data as u16;
                self.pulse_1.period_initial = (self.pulse_1.period_initial & 0xFF00) | period_low
            },
            0x4003 => {
                let period_high = ((data & 0b0000_0111) as u16) << 8;
                let length =     (data & 0b1111_1000) >> 3;

                self.pulse_1.period_initial = (self.pulse_1.period_initial & 0x00FF) | period_high;
                self.pulse_1.length = length;

                // Start this note
                self.pulse_1.sequence_counter = 0;
                self.pulse_1.envelope_start = true;
            },
            0x4004 => {
                let duty_index =      (data & 0b1100_0000) >> 6;
                let length_disable =  (data & 0b0010_0000) != 0;
                let constant_volume = (data & 0b0001_0000) != 0;

                self.pulse_2.duty = duty_table[duty_index as usize];
                self.pulse_2.length_enabled = !(length_disable);
                self.pulse_2.envelope_enabled = !(constant_volume);
                if constant_volume {
                    self.pulse_2.volume = data & 0b0000_1111;
                }
            },
            0x4005 => {
                self.pulse_2.sweep_enabled =  (data & 0b1000_0000) != 0;
                self.pulse_2.sweep_period =   (data & 0b0111_0000) >> 4;
                self.pulse_2.sweep_negate =   (data & 0b0000_1000) != 0;
                self.pulse_2.sweep_shift =     data & 0b0000_0111;
                self.pulse_2.sweep_reload = true;
            },
            0x4006 => {
                let period_low = data as u16;
                self.pulse_2.period_initial = (self.pulse_2.period_initial & 0xFF00) | period_low
            },
            0x4007 => {
                let period_high = ((data & 0b0000_0111) as u16) << 8;
                let length =     (data & 0b1111_1000) >> 3;

                self.pulse_2.period_initial = (self.pulse_2.period_initial & 0x00FF) | period_high;
                self.pulse_2.length = length;

                // Start this note
                self.pulse_2.sequence_counter = 0;
                self.pulse_2.envelope_start = true;
            },
            0x4017 => {
                self.frame_sequencer_mode = (data & 0b1000_0000) >> 7;
                self.disable_interrupt =    (data & 0b0100_0000) != 0;
                self.frame_reset_delay = 4;
            }

            _ => ()
        }
    }

    // Note: this uses CPU clocks, NOT APU clocks! It's simpler to represent the half-clock
    // updates this way. Documentation: https://wiki.nesdev.com/w/index.php/APU_Frame_Counter

    pub fn clock_frame_sequencer(&mut self) {
        self.frame_sequencer += 1;

        if self.frame_reset_delay > 0 {
            self.frame_reset_delay -= 1;
            if self.frame_reset_delay == 0 {
                self.frame_sequencer = 0;
                if self.frame_sequencer_mode == 1 {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                }
            }
        }

        if self.frame_sequencer_mode == 0 {
            // 4-step sequence
            match self.frame_sequencer {
                7457 => self.clock_quarter_frame(),
                14913 => {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                },
                22371 => self.clock_quarter_frame(),
                29828 => self.frame_interrupt = true,
                29829 => {
                    self.frame_interrupt = true;
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                },
                29830 => {
                    self.frame_interrupt = true;
                    self.frame_sequencer = 0;
                },
                _ => ()
            }
        } else {
            match self.frame_sequencer {
                // "5-step" sequence (uneven timing)
                7457 => self.clock_quarter_frame(),
                14913 => {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                },
                22371 => self.clock_quarter_frame(),
                37281 => {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                },
                37282 => {
                    self.frame_sequencer = 0;
                },
                _ => ()
            }
        }
    }

    pub fn clock_quarter_frame(&mut self) {

    }

    pub fn clock_half_frame(&mut self) {
        self.pulse_1.update_sweep();
        self.pulse_2.update_sweep();
    }

    pub fn run_to_cycle(&mut self, target_cycle: u64) {
        // For testing: Pulse 1 only
        while self.current_cycle < target_cycle {
            self.clock_frame_sequencer();
            // Only clock Pulse channels on every other cycle
            if (self.current_cycle & 0b1) == 0 {
                self.pulse_1.clock();
                self.pulse_2.clock();
            }

            if self.current_cycle >= self.next_sample_at {
                // Mixing? Bah! Just throw the sample in the buffer.
                let mut composite_sample: i16 = 0;
                composite_sample += (self.pulse_1.output() as i16 - 8) * 512; // Sure, why not?
                composite_sample += (self.pulse_2.output() as i16 - 8) * 512;
                self.sample_buffer[self.buffer_index] = composite_sample;
                self.buffer_index = (self.buffer_index + 1) % self.sample_buffer.len();

                self.generated_samples += 1;
                self.next_sample_at = ((self.generated_samples + 1) * self.cpu_clock_rate) / self.sample_rate;

                if self.buffer_index == 0 {
                    self.dump_sample_buffer();
                }
            }

            self.current_cycle += 1;
        }
    }

    pub fn dump_sample_buffer(&self) {
        let mut file =
            OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open("audiodump.raw")
            .unwrap();

        // turn our sample buffer into a simple file buffer for output
        let mut buffer = [0u8; 4096 * 2];
        for i in 0 .. 4096 {
            buffer[i * 2]     = (((self.sample_buffer[i] as u16) & 0xFF00) >> 8) as u8;
            buffer[i * 2 + 1] = (((self.sample_buffer[i] as u16) & 0x00FF)     ) as u8;
        }

        file.write_all(&buffer);
    }
}
