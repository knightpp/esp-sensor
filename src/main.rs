#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use hal::{clock::ClockControl, peripherals::Peripherals, prelude::*, timer::TimerGroup, Rtc, IO};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let mut system = peripherals.DPORT.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    // Disable the RTC and TIMG watchdog timers
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt1 = timer_group1.wdt;
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();

    let mut delay = hal::Delay::new(&clocks);
    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let mut data_pin = io.pins.gpio15.into_open_drain_output();

    let mut clk = io.pins.gpio12.into_open_drain_output();
    let mut dio = io.pins.gpio13.into_open_drain_output();
    let mut screen_delay = delay.clone();
    let mut tm = tm1637::TM1637::new(&mut clk, &mut dio, &mut screen_delay);
    tm.init().unwrap();
    tm.clear().unwrap();
    tm.set_brightness(255).unwrap();

    loop {
        let value =
            match dht_hal_drv::dht_read(dht_hal_drv::DhtType::DHT22, &mut data_pin, &mut |d| {
                delay.delay_us(d);
            }) {
                Ok(value) => value,
                Err(err) => {
                    println!("Error: {:?}", err);
                    delay.delay_ms(2000u16);
                    continue;
                }
            };

        let temp = value.temperature();
        let hum = value.humidity();
        println!("Temperature: {:2.1}°C", temp);
        println!("Humidity:    {:2.1}%", hum);

        let digits = [
            ((temp / 10.) as u32 % 10) as u8,
            (temp as u32 % 10) as u8,
            ((hum / 10.) as u32 % 10) as u8,
            (hum as u32 % 10) as u8,
        ];
        tm.print_hex(0, &digits).unwrap();
        delay.delay_ms(5000u16);
    }
}
