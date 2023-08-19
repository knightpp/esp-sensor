use bus::Bus;
use esp_idf_hal::{
    delay::{self},
    gpio::{self, PinDriver},
    prelude::Peripherals,
};
use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use std::{fmt::Display, thread, time::Duration};

#[derive(Debug)]
#[toml_cfg::toml_config]
pub struct Config {
    #[default("<CHANGEME>")]
    ssid: &'static str,
    #[default("<CHANGEME>")]
    password: &'static str,
    #[default("<CHANGEME>")]
    addr: &'static str,
    #[default("<CHANGEME>")]
    influx_token: &'static str,
    #[default("<CHANGEME>")]
    influx_org: &'static str,
    #[default("<CHANGEME>")]
    influx_bucket: &'static str,
    #[default(30)]
    read_sensor_interval_secs: u32,
}

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("using {:?}", CONFIG);

    let mut bus = bus::Bus::<SensorData>::new(4);
    let sub1 = bus.add_rx();

    let peripherals = Peripherals::take().unwrap();
    let dht22_pin = PinDriver::input_output(peripherals.pins.gpio15).unwrap();
    let display_clk = PinDriver::input_output(peripherals.pins.gpio32).unwrap();
    let display_dio = PinDriver::input_output(peripherals.pins.gpio33).unwrap();

    thread::scope(|s| {
        s.spawn(|| read_sensor(&mut bus, dht22_pin));
        s.spawn(|| display_sensor_data(sub1, display_clk, display_dio));
    });
}

fn read_sensor<P: gpio::InputPin + gpio::OutputPin>(
    bus: &mut Bus<SensorData>,
    mut pin: PinDriver<'_, P, gpio::InputOutput>,
) {
    thread::sleep(Duration::from_secs(3));

    loop {
        let value =
            match dht_hal_drv::dht_read(dht_hal_drv::DhtType::DHT22, &mut pin, delay::FreeRtos) {
                Result::Ok(x) => x,
                Result::Err(err) => {
                    log::error!("read_sensor: reading dht sensor error={:?}", err);
                    log::trace!("read_sensor: going to sleep for 10s...");
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

        let value: SensorData = value.into();
        log::info!("read_sensor: data={}", value);
        bus.broadcast(value);
        let interval = CONFIG.read_sensor_interval_secs;
        log::trace!("read_sensor: sleeping for {}s...", interval);
        thread::sleep(Duration::from_secs(interval as u64));
    }
}

fn display_sensor_data<'d, PCLK, PDIO>(
    mut sub: bus::BusReader<SensorData>,
    clk: PinDriver<'d, PCLK, gpio::InputOutput>,
    dio: PinDriver<'d, PDIO, gpio::InputOutput>,
) where
    PCLK: gpio::InputPin + gpio::OutputPin,
    PDIO: gpio::InputPin + gpio::OutputPin,
{
    thread::sleep(Duration::from_secs(5));

    let mut tm = tm1637::TM1637::new(clk, dio, delay::Ets);
    log::trace!("init tm1637...");
    tm.init().unwrap();
    log::trace!("clear tm1637...");
    tm.clear().unwrap();
    log::trace!("set brightness tm1637...");
    tm.set_brightness(128).unwrap();

    for SensorData {
        temperature,
        humidity,
    } in sub.iter()
    {
        let digits = [
            ((temperature / 10.) as u32 % 10) as u8,
            (temperature as u32 % 10) as u8,
            ((humidity / 10.) as u32 % 10) as u8,
            (humidity as u32 % 10) as u8,
        ];

        log::trace!("displaying data on tm1637...");
        if let Err(err) = tm.print_hex(0, &digits) {
            log::error!("failed to print hex on tm1637 error={:?}", err);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SensorData {
    temperature: f32,
    humidity: f32,
}

impl Display for SensorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "temperature={:2.1}°C humidity={:2.1}%",
            self.temperature, self.humidity
        ))
    }
}

impl From<dht_hal_drv::DhtValue> for SensorData {
    fn from(value: dht_hal_drv::DhtValue) -> Self {
        Self {
            temperature: value.temperature(),
            humidity: value.humidity(),
        }
    }
}
