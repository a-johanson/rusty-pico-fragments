//! Waveshare Pico LCD 2 Display Driver
//!
//! Driver for the Waveshare Pico LCD 2 inch display with ST7789 controller, 
//! integrated with RP2350 DMA for double buffering.

use embedded_hal::digital::OutputPin;
use embedded_hal::delay::DelayNs;
use embedded_hal::spi::SpiBus;
use rp235x_hal::dma::single_buffer;
use rp235x_hal::dma::SingleChannel;
use rp235x_hal::dma::WriteTarget;
use rp235x_hal::singleton;


pub const WIDTH: u16 = 240;
pub const HEIGHT: u16 = 320;
const BUFFER_SIZE: usize = (WIDTH as usize) * (HEIGHT as usize) * 3;

/// ST7789VW Commands
#[repr(u8)]
enum Command {
    SwReset = 0x01,
    SlpOut = 0x11,
    InvOn = 0x21,
    DispOn = 0x29,
    CaSet = 0x2A,
    RaSet = 0x2B,
    RamWr = 0x2C,
    MadCtl = 0x36,
    ColMod = 0x3A,
    // PorCtrl = 0xB2,
    // GCtrl = 0xB7,
    // VcomS = 0xBB,
    // LcmCtrl = 0xC0,
    // VdvVrhEn = 0xC2,
    // VrhSet = 0xC3,
    // VdvSet = 0xC4,
    // FrCtrl2 = 0xC6,
    // PwCtrl1 = 0xD0,
    PvGamCtrl = 0xE0,
    NvGamCtrl = 0xE1,
}


pub struct WaveshareST7789Display<SPI: WriteTarget<TransmittedWord = u8> + SpiBus, CS: OutputPin, DC: OutputPin, RST: OutputPin, DMACH: SingleChannel> {
    spi: Option<SPI>,
    cs: CS,
    dc: DC,
    rst: RST,
    dma_ch: Option<DMACH>,
    transfer: Option<single_buffer::Transfer<DMACH, &'static mut [u8; BUFFER_SIZE], SPI>>,
}

impl<SPI: WriteTarget<TransmittedWord = u8> + SpiBus, CS: OutputPin, DC: OutputPin, RST: OutputPin, DMACH: SingleChannel> WaveshareST7789Display<SPI, CS, DC, RST, DMACH> {
    /// Create a new display driver with DMA support
    pub fn new(
        spi: SPI, 
        cs: CS, 
        dc: DC, 
        rst: RST, 
        dma_ch: DMACH
    ) -> Self {
        Self {
            spi: Some(spi),
            cs,
            dc,
            rst,
            dma_ch: Some(dma_ch),
            transfer: None,
        }
    }

    /// Initialize the display and start first DMA transfer
    /// Returns the idle buffer for the user to fill
    pub fn init<DELAY: DelayNs>(&mut self, delay: &mut DELAY) -> &'static mut [u8; BUFFER_SIZE] {
        let _ = self.cs.set_high();
        self.hard_reset(delay);

        self.write_command(delay, Command::SwReset);
        delay.delay_ms(150);

        self.write_command(delay, Command::SlpOut);
        delay.delay_ms(150);

        self.write_command(delay, Command::ColMod); 
        self.write_data(delay, &[0x06]);

        self.write_command(delay, Command::MadCtl);
        self.write_data(delay, &[0x00]);

        self.write_command(delay, Command::InvOn); 

        self.write_command(delay, Command::CaSet);
        let cols = WIDTH - 1;
        self.write_data(delay, &[0x00, 0x00, (cols >> 8) as u8, (cols & 0xFF) as u8]);

        self.write_command(delay, Command::RaSet);
        let rows = HEIGHT - 1;
        self.write_data(delay, &[0x00, 0x00, (rows >> 8) as u8, (rows & 0xFF) as u8]);

        self.write_command(delay, Command::PvGamCtrl);
        self.write_data(delay, &[0xD0, 0x08, 0x11, 0x08, 0x0C, 0x15, 0x39, 0x33, 0x50, 0x36, 0x13, 0x14, 0x29, 0x2D]);

        self.write_command(delay, Command::NvGamCtrl);
        self.write_data(delay, &[0xD0, 0x08, 0x10, 0x08, 0x06, 0x06, 0x39, 0x44, 0x51, 0x0B, 0x16, 0x14, 0x2F, 0x31]);

        // Allocate two buffers for double buffering
        let buffer_a = singleton!(: [u8; BUFFER_SIZE] = [0u8; BUFFER_SIZE]).unwrap();
        let buffer_b = singleton!(: [u8; BUFFER_SIZE] = [0u8; BUFFER_SIZE]).unwrap();
        self.write_command(delay, Command::RamWr);
        delay.delay_ms(1);
        self.write_data(delay, buffer_a);
        delay.delay_ms(1);

        self.write_command(delay, Command::DispOn);
        delay.delay_ms(120);

        // Start first DMA transfer with buffer_a (all zeros/black)
        let mut spi = self.spi.take().unwrap();
        let ch = self.dma_ch.take().unwrap();

        // Send RAMWR command and prepare for DMA
        self.start_frame(delay, &mut spi);
        
        // Start DMA transfer with buffer_a 
        let transfer = single_buffer::Config::new(ch, buffer_a, spi).start();
        self.transfer = Some(transfer);
        
        // Return buffer_b for user to fill while buffer_a is being transferred
        buffer_b
    }

    /// Swap buffers: submit filled buffer for DMA transfer and get the other buffer back
    /// 
    /// This achieves true parallelism:
    /// 1. Wait for current transfer to complete
    /// 2. Start new DMA transfer with the ready_buffer you provide
    /// 3. Return the buffer that just finished for user to fill
    pub fn swap_buffers<DELAY: DelayNs>(&mut self, delay: &mut DELAY, ready_buffer: &'static mut [u8; BUFFER_SIZE]) -> &'static mut [u8; BUFFER_SIZE] {
        // Step 1: Wait for current transfer to complete
        let transfer = self.transfer.take().unwrap();
        let (ch, completed_buffer, mut spi) = transfer.wait();
        
        let _ = self.cs.set_high();
        delay.delay_ms(1);

        // Step 2: Start new transfer with the ready_buffer user just gave us
        // Send RAMWR command for next frame
        self.start_frame(delay, &mut spi);
        
        // Start DMA transfer with ready_buffer
        let transfer = single_buffer::Config::new(ch, ready_buffer, spi).start();
        self.transfer = Some(transfer);
        
        // Step 3: Return the completed_buffer for user to fill while DMA runs
        completed_buffer
    }

    /// Hardware reset the display
    pub fn hard_reset<DELAY: DelayNs>(&mut self, delay: &mut DELAY) {
        let _ = self.rst.set_high();
        delay.delay_ms(100);
        let _ = self.rst.set_low();
        delay.delay_ms(100);
        let _ = self.rst.set_high();
        delay.delay_ms(120);
    }

    /// Write a command to the display
    fn start_frame<DELAY: DelayNs>(&mut self, delay: &mut DELAY, spi: &mut SPI) {
        let _ = self.dc.set_low(); // Command mode
        let _ = self.cs.set_low(); // Select the display
        delay.delay_ns(100);
        let _ = spi.write(&[Command::RamWr as u8]);
        delay.delay_ns(100);
        let _ = self.dc.set_high();
        delay.delay_ms(1);
    }

    /// Write a command to the display
    fn write_command<DELAY: DelayNs>(&mut self, delay: &mut DELAY, command: Command) {
        let _ = self.dc.set_low(); // Command mode
        let _ = self.cs.set_low(); // Select the display
        delay.delay_ns(100);
        let _ = self.spi.as_mut().unwrap().write(&[command as u8]);
        let _ = self.cs.set_high(); // Deselect the display
        delay.delay_ns(100);
    }

    /// Write data to the display
    fn write_data<DELAY: DelayNs>(&mut self, delay: &mut DELAY, data: &[u8]) {
        let _ = self.dc.set_high(); // Data mode
        let _ = self.cs.set_low(); // Select the display
        delay.delay_ns(100);
        let _ = self.spi.as_mut().unwrap().write(data);
        let _ = self.cs.set_high(); // Deselect the display
        delay.delay_ns(100);
    }
}
