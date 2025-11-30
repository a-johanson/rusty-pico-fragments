#![no_std]
#![no_main]

mod display;

use embedded_hal::digital::StatefulOutputPin;
use panic_halt as _;
use rp235x_hal::clocks::init_clocks_and_plls;
use rp235x_hal::gpio::PinState;
use rp235x_hal::{self as hal, entry};
use rp235x_hal::{Clock, pac};
use rp235x_hal::dma::DMAExt;

use fugit::RateExtU32;

use display::WaveshareST7789Display;

/// Fill a frame buffer with a color gradient
fn fill_frame_buffer(buffer: &mut [u8], frame_count: u32, width: usize, height: usize) {
    for y in 0..height {
        let v = y as f32 / ((height - 1) as f32);
        let g = (v * 255.0f32) as u8 & 0xFC;
        let b = (frame_count & 0xFC) as u8;
        for x in 0..width {
            let u = x as f32 / ((width - 1) as f32);
            let r = (u * 255.0f32) as u8 & 0xFC;
            let base_index = 3 * (y * width + x);
            buffer[base_index] = r;
            buffer[base_index + 1] = g;
            buffer[base_index + 2] = b;
        }
    }
}

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();


#[entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let _core = cortex_m::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let sio = hal::Sio::new(pac.SIO);

    // External high-speed crystal on the Pico 2 board is 12 MHz
    let external_xtal_freq_hz = 12_000_000u32;
    let clocks = init_clocks_and_plls(
        external_xtal_freq_hz,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);
    let mut delay_for_app = timer.clone();

    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    
    let lcd_clk = pins.gpio10.into_function::<hal::gpio::FunctionSpi>();
    let lcd_din = pins.gpio11.into_function::<hal::gpio::FunctionSpi>();
    let lcd_cs = pins.gpio9.into_push_pull_output_in_state(PinState::High);
    let lcd_dc = pins.gpio8.into_push_pull_output_in_state(PinState::Low);
    let lcd_rst = pins.gpio12.into_push_pull_output_in_state(PinState::High);
    let _lcd_bl = pins.gpio13.into_push_pull_output_in_state(PinState::High);
    let mut led_pin = pins.gpio25.into_push_pull_output_in_state(PinState::High);

    // Configure SPI
    let spi = hal::spi::Spi::<_, _, _, 8>::new(pac.SPI1, (lcd_din, lcd_clk));
    
    // ST7789 can support up to 62.5 MHz, but start conservatively at 16 MHz
    let spi = spi.init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        150u32.MHz(),
        embedded_hal::spi::MODE_0,
    );
    
    // Initialize DMA
    let dma = pac.DMA.split(&mut pac.RESETS);
    
    let mut display = WaveshareST7789Display::new(spi, lcd_cs, lcd_dc, lcd_rst, dma.ch0);

    // Initialize the display and get first buffer to fill
    let mut buffer = display.init(&mut delay_for_app);

    let mut frame_count = 0u32;
    
    // Main rendering loop with double buffering
    loop {
        // Fill the buffer we have
        fill_frame_buffer(buffer, frame_count, display::WIDTH as usize, display::HEIGHT as usize);
        
        // Swap: submit filled buffer for DMA transfer, get the other buffer back
        buffer = display.swap_buffers(&mut delay_for_app, buffer);
        
        // Toggle LED to show activity
        if frame_count % 30 == 0 {
            let _ = led_pin.toggle();
        }
        
        frame_count += 1;
    }
}

/// Program metadata for `picotool info`
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [rp235x_hal::binary_info::EntryAddr; 5] = [
    rp235x_hal::binary_info::rp_cargo_bin_name!(),
    rp235x_hal::binary_info::rp_cargo_version!(),
    rp235x_hal::binary_info::rp_program_description!(c"rusty-pico-fragments"),
    rp235x_hal::binary_info::rp_cargo_homepage_url!(),
    rp235x_hal::binary_info::rp_program_build_attribute!(),
];
