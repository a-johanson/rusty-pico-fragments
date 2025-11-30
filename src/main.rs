#![no_std]
#![no_main]

mod display;

use embedded_hal::digital::StatefulOutputPin;
use panic_halt as _;
use rp235x_hal::gpio::PinState;
use rp235x_hal::{self as hal, entry};
use rp235x_hal::pac;
use rp235x_hal::dma::DMAExt;
use rp235x_hal::clocks::{Clock, ClocksManager, ClockSource, InitError};
use rp235x_hal::pll::{PLLConfig, common_configs::{PLL_USB_48MHZ}, setup_pll_blocking};
use rp235x_hal::Sio;
use rp235x_hal::watchdog::Watchdog;
use rp235x_hal::xosc::setup_xosc_blocking;


use fugit::{RateExtU32, HertzU32};

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
    let mut peripherals = pac::Peripherals::take().unwrap();
    let _core = cortex_m::Peripherals::take().unwrap();
    let mut watchdog = Watchdog::new(peripherals.WATCHDOG);

    // External high-speed crystal on the Pico 2 board is 12 MHz
    const XOSC_CRYSTAL_FREQ: u32 = 12_000_000; 

    // Enable the xosc
    let xosc = setup_xosc_blocking(peripherals.XOSC, XOSC_CRYSTAL_FREQ.Hz()).map_err(InitError::XoscErr).unwrap();

    // Start tick in watchdog
    watchdog.enable_tick_generation((XOSC_CRYSTAL_FREQ / 1_000_000) as u16);

    let mut clocks = ClocksManager::new(peripherals.CLOCKS);

    // Configure PLLs
    const PLL_SYS_250MHZ: PLLConfig = PLLConfig {
        vco_freq: HertzU32::MHz(1500),
        refdiv: 1,
        post_div1: 3,
        post_div2: 2,
    };
    let pll_sys = setup_pll_blocking(peripherals.PLL_SYS, xosc.operating_frequency().into(), PLL_SYS_250MHZ, &mut clocks, &mut peripherals.RESETS).map_err(InitError::PllError).unwrap();
    let pll_usb = setup_pll_blocking(peripherals.PLL_USB, xosc.operating_frequency().into(), PLL_USB_48MHZ, &mut clocks, &mut peripherals.RESETS).map_err(InitError::PllError).unwrap();

    // Configure clocks
    // CLK_REF = XOSC (12MHz) / 1 = 12MHz
    clocks.reference_clock.configure_clock(&xosc, xosc.get_freq()).map_err(InitError::ClockError).unwrap();

    // CLK SYS = PLL SYS (250MHz) / 1 = 250MHz
    clocks.system_clock.configure_clock(&pll_sys, pll_sys.get_freq()).map_err(InitError::ClockError).unwrap();

    // CLK USB = PLL USB (48MHz) / 1 = 48MHz
    clocks.usb_clock.configure_clock(&pll_usb, pll_usb.get_freq()).map_err(InitError::ClockError).unwrap();

    // CLK ADC = PLL USB (48MHZ) / 1 = 48MHz
    clocks.adc_clock.configure_clock(&pll_usb, pll_usb.get_freq()).map_err(InitError::ClockError).unwrap();

    // CLK HSTX = PLL SYS (250MHz) / 1 = 250MHz
    clocks.hstx_clock.configure_clock(&pll_sys, pll_sys.get_freq()).map_err(InitError::ClockError).unwrap();

    // CLK PERI = clk_sys. Used as reference clock for Peripherals. No dividers so just select and enable
    clocks.peripheral_clock.configure_clock(&clocks.system_clock, clocks.system_clock.freq()).map_err(InitError::ClockError).unwrap();

    let timer = hal::Timer::new_timer0(peripherals.TIMER0, &mut peripherals.RESETS, &clocks);
    let mut delay_for_app = timer.clone();

    let sio = Sio::new(peripherals.SIO);
    let pins = hal::gpio::Pins::new(
        peripherals.IO_BANK0,
        peripherals.PADS_BANK0,
        sio.gpio_bank0,
        &mut peripherals.RESETS,
    );
    
    let lcd_clk = pins.gpio10.into_function::<hal::gpio::FunctionSpi>();
    let lcd_din = pins.gpio11.into_function::<hal::gpio::FunctionSpi>();
    let lcd_cs = pins.gpio9.into_push_pull_output_in_state(PinState::High);
    let lcd_dc = pins.gpio8.into_push_pull_output_in_state(PinState::Low);
    let lcd_rst = pins.gpio12.into_push_pull_output_in_state(PinState::High);
    let _lcd_bl = pins.gpio13.into_push_pull_output_in_state(PinState::High);
    let mut led_pin = pins.gpio25.into_push_pull_output_in_state(PinState::High);

    // Configure SPI
    let spi = hal::spi::Spi::<_, _, _, 8>::new(peripherals.SPI1, (lcd_din, lcd_clk));
    
    // ST7789 can support up to 62.5 MHz, but start conservatively at 16 MHz
    let spi = spi.init(
        &mut peripherals.RESETS,
        clocks.peripheral_clock.freq(),
        250u32.MHz(),
        embedded_hal::spi::MODE_0,
    );
    
    // Initialize DMA
    let dma = peripherals.DMA.split(&mut peripherals.RESETS);
    
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
