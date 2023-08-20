mod line_proto;

use anyhow::{bail, Context};
use bus::Bus;
use embedded_svc::{
    http::client::Client,
    wifi::{AuthMethod, ClientConfiguration, Configuration},
};
use esp_idf_hal::{
    delay::{self},
    gpio::{self, PinDriver},
    peripheral,
    prelude::Peripherals,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, http::client::Configuration as HttpConfiguration,
    http::client::EspHttpConnection, wifi::BlockingWifi, wifi::EspWifi,
};
use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use std::{convert::Infallible, fmt::Display, thread, time::Duration};

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
    let sub2 = bus.add_rx();

    let sysloop = EspSystemEventLoop::take().unwrap();
    let mut peripherals = Peripherals::take().unwrap();

    let dht22_pin = PinDriver::input_output(peripherals.pins.gpio15).unwrap();
    let display_clk = PinDriver::input_output(peripherals.pins.gpio32).unwrap();
    let display_dio = PinDriver::input_output(peripherals.pins.gpio33).unwrap();

    thread::scope(|s| {
        s.spawn(|| read_sensor(&mut bus, dht22_pin));
        s.spawn(|| display_sensor_data(sub1, display_clk, display_dio));
        s.spawn(|| data_sender(sub2, &mut peripherals.modem, &sysloop));
    });
}

fn data_sender(
    mut sub: bus::BusReader<SensorData>,
    modem: &mut impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem>,
    sysloop: &EspSystemEventLoop,
) {
    loop {
        if let Err(err) = data_sender_inner(&mut sub, modem, sysloop) {
            log::error!("could not send sensor data error={}", err);
        }

        thread::sleep(Duration::from_secs(30));
    }
}

fn data_sender_inner(
    sub: &mut bus::BusReader<SensorData>,
    modem: &mut impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem>,
    sysloop: &EspSystemEventLoop,
) -> anyhow::Result<Infallible> {
    let _wifi = wifi(modem, sysloop.clone()).context("connect to wi-fi")?;
    log::info!("Connected to Wi-Fi network!");

    let http_connection = EspHttpConnection::new(&HttpConfiguration::default())?;
    let mut client = Client::wrap(http_connection);
    let addr = format!(
        "{}/api/v2/write?org={}&bucket={}&precision=ns",
        CONFIG.addr, CONFIG.influx_org, CONFIG.influx_bucket
    );

    for data in sub.iter() {
        let mut request = client
            .post(
                &addr,
                &[
                    ("Authorization", CONFIG.influx_token),
                    ("Accept", "application/json"),
                    ("Content-Type", "text/plain"),
                ],
            )
            .context("create post request")?;

        line_proto::new(&mut request)
            .measurement("dht22")?
            .next()?
            .field("humidity", data.humidity)?
            .field("temperature", data.temperature)?
            .next()
            .build()?;

        log::trace!("doing http post request...");
        let mut response = request.submit().context("do post request")?;
        let status = response.status();
        if (200..300).contains(&status) {
            log::trace!("http post success!");
        } else {
            let mut buf = {
                response
                    .header("Content-Length")
                    .map_or_else(Vec::<u8>::new, |len| {
                        Vec::<u8>::with_capacity(len.parse().ok().unwrap_or(0))
                    })
            };

            loop {
                match response.read(&mut buf) {
                    Ok(n) => {
                        if n == 0 {
                            break;
                        }
                    }
                    Err(err) => {
                        log::error!("failed to read http response error={}", err);
                        break;
                    }
                }
            }

            if let Ok(body) = std::str::from_utf8(&buf) {
                log::error!("non 2XX http response body={body}")
            } else {
                log::error!("non 2XX http response")
            };
        }
    }

    bail!("subscription drained")
}

fn read_sensor<P: gpio::InputPin + gpio::OutputPin>(
    bus: &mut Bus<SensorData>,
    mut pin: PinDriver<'_, P, gpio::InputOutput>,
) {
    thread::sleep(Duration::from_secs(3));

    loop {
        let value = match dht_hal_drv::dht_read(dht_hal_drv::DhtType::DHT22, &mut pin, delay::Ets) {
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
        thread::sleep(Duration::from_secs(u64::from(interval)));
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

fn wifi(
    modem: &'_ mut impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem>,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<Box<EspWifi<'_>>> {
    let auth_method = AuthMethod::WPA3Personal;
    let ssid = CONFIG.ssid;
    let pass = CONFIG.password;
    if ssid.is_empty() {
        bail!("Missing WiFi name")
    }
    if pass.is_empty() {
        bail!("Missing WiFi password")
    }

    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;
    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;

    log::info!("Starting wifi...");

    wifi.start()?;

    log::info!("Scanning...");

    let ap_infos = wifi.scan()?;

    let ours = ap_infos.into_iter().find(|a| a.ssid == ssid);

    let channel = if let Some(ours) = ours {
        log::info!(
            "Found configured access point {} on channel {}",
            ssid,
            ours.channel
        );
        Some(ours.channel)
    } else {
        log::info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
        None
    };

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: ssid.into(),
        password: pass.into(),
        channel,
        auth_method,
        ..Default::default()
    }))?;

    log::info!("Connecting wifi...");

    wifi.connect()?;

    log::info!("Waiting for DHCP lease...");

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    log::info!("Wifi DHCP info: {:?}", ip_info);

    Ok(Box::new(esp_wifi))
}
