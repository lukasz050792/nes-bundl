use crate::apu::ApuState;
use crate::cartridge;
use crate::cycle_cpu;
use crate::cycle_cpu::CpuState;
use crate::cycle_cpu::Registers;
use crate::memory;
use crate::memory::CpuMemory;
use crate::ppu::PpuState;
use crate::mmc::mapper::Mapper;
use crate::save_load::*;
use crate::tracked_events::EventTracker;

pub struct NesState {
    pub apu: ApuState,
    pub cpu: CpuState,
    pub memory: CpuMemory,
    pub ppu: PpuState,
    pub registers: Registers,
    pub master_clock: u64,
    pub p1_input: u8,
    pub p1_data: u8,
    pub p2_input: u8,
    pub p2_data: u8,
    pub input_latch: bool,
    pub mapper: Box<dyn Mapper>,
    pub last_frame: u32,
    pub event_tracker: EventTracker,
}

impl NesState {
    pub fn new(m: Box<dyn Mapper>) -> NesState {
        return NesState {
            apu: ApuState::new(),
            cpu: CpuState::new(),
            memory: CpuMemory::new(),
            ppu: PpuState::new(),
            registers: Registers::new(),
            master_clock: 0,
            p1_input: 0,
            p1_data: 0,
            p2_input: 0,
            p2_data: 0,
            input_latch: false,
            mapper: m,
            last_frame: 0,
            event_tracker: EventTracker::new(),
        }
    }

    pub fn save_state(&self) -> Vec<u8> {
        let mut buff = vec!();
        self.apu.save_state(&mut buff);
        self.cpu.save_state(&mut buff);
        self.memory.save_state(&mut buff);
        self.ppu.save_state(&mut buff);
        self.registers.save_state(&mut buff);
        save_u64(&mut buff, self.master_clock);
        save_u8(&mut buff, self.p1_input);
        save_u8(&mut buff, self.p1_data);
        save_u8(&mut buff, self.p2_input);
        save_u8(&mut buff, self.p2_data);
        save_bool(&mut buff, self.input_latch);
        self.mapper.save_state(&mut buff);
        save_u32(&mut buff, self.last_frame);
        buff
    }

    pub fn load_state(&mut self, buff: &mut Vec<u8>) {
        load_u32(buff, &mut self.last_frame);
        self.mapper.load_state(buff);
        load_bool(buff, &mut self.input_latch);
        load_u8(buff, &mut self.p2_data);
        load_u8(buff, &mut self.p2_input);
        load_u8(buff, &mut self.p1_data);
        load_u8(buff, &mut self.p1_input);
        load_u64(buff, &mut self.master_clock);
        self.registers.load_state(buff);
        self.ppu.load_state(buff);
        self.memory.load_state(buff);
        self.cpu.load_state(buff);
        self.apu.load_state(buff);
    }

    #[deprecated(since="0.2.0", note="please use `::new(mapper)` instead")]
    pub fn from_rom(cart_data: &[u8]) -> Result<NesState, String> {
        let maybe_mapper = cartridge::mapper_from_file(cart_data);
        match maybe_mapper {
            Ok(mapper) => {
                let mut nes = NesState::new(mapper);
                nes.power_on();
                return Ok(nes);
            },
            Err(why) => {
                return Err(why);
            }
        }
    }

    pub fn power_on(&mut self) {
        // Initialize CPU register state for power-up sequence
        self.registers.a = 0;
        self.registers.y = 0;
        self.registers.x = 0;
        self.registers.s = 0xFD;

        self.registers.set_status_from_byte(0x34);

        // Initialize I/O and Audio registers to known startup values
        for i in 0x4000 .. (0x400F + 1) {
            memory::write_byte(self, i, 0);
        }
        memory::write_byte(self, 0x4015, 0);
        memory::write_byte(self, 0x4017, 0);

        let pc_low = memory::read_byte(self, 0xFFFC);
        let pc_high = memory::read_byte(self, 0xFFFD);
        self.registers.pc = pc_low as u16 + ((pc_high as u16) << 8);

        // Clock the APU 10 times (this subtly affects the first IRQ's timing and frame counter operation)
        for _ in 0 .. 10 {
            self.apu.clock_apu(&mut *self.mapper);
        }
    }

    pub fn reset(&mut self) {
        self.registers.s = self.registers.s.wrapping_sub(3);
        self.registers.flags.interrupts_disabled = true;

        // Silence the APU
        memory::write_byte(self, 0x4015, 0);

        let pc_low = memory::read_byte(self, 0xFFFC);
        let pc_high = memory::read_byte(self, 0xFFFD);
        self.registers.pc = pc_low as u16 + ((pc_high as u16) << 8);
    }

    pub fn cycle(&mut self) {
        cycle_cpu::run_one_clock(self);
        self.master_clock = self.master_clock + 12;
        // Three PPU clocks per every 1 CPU clock
        self.ppu.clock(&mut *self.mapper);
        self.ppu.clock(&mut *self.mapper);
        self.ppu.clock(&mut *self.mapper);
        self.event_tracker.current_scanline = self.ppu.current_scanline;
        self.event_tracker.current_cycle = self.ppu.current_scanline_cycle;
        self.apu.clock_apu(&mut *self.mapper);
        self.mapper.clock_cpu();
    }

    pub fn step(&mut self) {
        // Always run at least one cycle
        self.cycle();
        let mut i = 0;
        // Continue until either we loop back around to cycle 0 (a new instruction)
        // or this instruction has failed to reset (encountered a STP or an opcode bug)
        while self.cpu.tick >= 1 && i < 10 {
            self.cycle();
            i += 1;
        }
        if self.ppu.current_frame != self.last_frame {
            self.event_tracker.swap_buffers();
            self.last_frame = self.ppu.current_frame;
        }
    }

    pub fn run_until_hblank(&mut self) {
        let old_scanline = self.ppu.current_scanline;
        while old_scanline == self.ppu.current_scanline {
            self.step();
        }
    }

    pub fn run_until_vblank(&mut self) {
        while self.ppu.current_scanline == 242 {
            self.step();
        }
        while self.ppu.current_scanline != 242 {
            self.step();
        }
    }

    pub fn nudge_ppu_alignment(&mut self) {
        // Give the PPU a swift kick:
        self.ppu.clock(&mut *self.mapper);
        self.event_tracker.current_scanline = self.ppu.current_scanline;
        self.event_tracker.current_cycle = self.ppu.current_scanline_cycle;
    }

    pub fn sram(&self) -> Vec<u8> {
        return self.mapper.get_sram();
    }

    pub fn set_sram(&mut self, sram_data: Vec<u8>) {
        if sram_data.len() != self.mapper.get_sram().len() {
            println!("SRAM size mismatch, expected {} bytes but file is {} bytes!", self.mapper.get_sram().len(), sram_data.len());
        } else {
            self.mapper.load_sram(sram_data);
        }
    }
}
