use anyhow::{bail, Context};
use bus::Bus;
use embedded_svc::{
    http::client::{Client, Response},
    io::Write,
    utils::io,
    wifi::{ClientConfiguration, Configuration},
};
use esp_idf_hal::{
    delay::{self},
    gpio::{self, PinDriver},
    peripheral,
    prelude::Peripherals,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, http::client::Configuration as HttpConfiguration,
    http::client::EspHttpConnection, nvs::EspDefaultNvsPartition, wifi::BlockingWifi,
    wifi::EspWifi,
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

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    let logger = esp_idf_svc::log::EspLogger;
    logger.set_target_level("esp_sensor", log::LevelFilter::Trace)?;

    log::info!("using {:?}", CONFIG);
    let mut bus = bus::Bus::<SensorData>::new(4);
    let sub1 = bus.add_rx();
    let sub2 = bus.add_rx();

    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut peripherals = Peripherals::take().context("no peripherals")?;

    let dht22_pin = PinDriver::input_output(peripherals.pins.gpio3)?;
    let display_clk = PinDriver::input_output(peripherals.pins.gpio1)?;
    let display_dio = PinDriver::input_output(peripherals.pins.gpio10)?;

    thread::scope(|s| {
        s.spawn(|| read_sensor(&mut bus, dht22_pin));
        s.spawn(|| display_sensor_data(sub1, display_clk, display_dio));
        s.spawn(|| data_sender(sub2, &mut peripherals.modem, &sysloop, Some(nvs)));
    });

    Ok(())
}

fn data_sender(
    mut sub: bus::BusReader<SensorData>,
    modem: &mut impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem>,
    sysloop: &EspSystemEventLoop,
    nvs: Option<EspDefaultNvsPartition>,
) {
    loop {
        if let Err(err) = data_sender_inner(&mut sub, modem, sysloop, nvs.clone()) {
            log::error!("could not send sensor data error={:?}", err);
        }

        thread::sleep(Duration::from_secs(30));
    }
}

fn data_sender_inner(
    sub: &mut bus::BusReader<SensorData>,
    modem: &mut impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem>,
    sysloop: &EspSystemEventLoop,
    nvs: Option<EspDefaultNvsPartition>,
) -> anyhow::Result<Infallible> {
    let _wifi = wifi(modem, sysloop.clone(), nvs).context("connect to wi-fi")?;
    log::info!("Connected to Wi-Fi network!");

    let http_connection = EspHttpConnection::new(&HttpConfiguration {
        timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    })
    .context("create esp http connection")?;
    let mut client = Client::wrap(http_connection);
    let addr = format!(
        "{}/api/v2/write?org={}&bucket={}&precision=ns",
        CONFIG.addr, CONFIG.influx_org, CONFIG.influx_bucket
    );

    log::info!("http API addr={}", addr);

    let token = format!("Token {}", CONFIG.influx_token);
    for data in sub.iter() {
        do_request(&mut client, &addr, &token, data)?;
    }

    bail!("subscription drained")
}

fn handle_response(response: Response<&mut EspHttpConnection>) -> Result<(), anyhow::Error> {
    let status = response.status();
    if (200..300).contains(&status) {
        log::trace!("http post success!");
    } else {
        log::error!(
            "http status code={} status={:?}",
            status,
            response.status_message()
        );
    }
    read_body(response)?;
    Ok(())
}

fn do_request(
    client: &mut Client<EspHttpConnection>,
    addr: &str,
    token: &str,
    data: SensorData,
) -> Result<(), anyhow::Error> {
    let mut body = influxdb_line_protocol::builder::LineProtocolBuilder::new()
        .measurement("living room #1")
        .tag("sensor", "dht22")
        .field("humidity", data.humidity as f64)
        .field("temperature", data.temperature as f64)
        .close_line()
        .build();
    body.shrink_to_fit();
    let content_length_header = format!("{}", body.len());
    let headers = [
        ("authorization", token),
        ("accept", "application/json"),
        ("content-type", "text/plain"),
        ("connection", "keep-alive"),
        ("content-length", &*content_length_header),
    ];

    let mut request = client.post(addr, &headers).context("create post request")?;
    request.write_all(&body)?;
    request.flush()?;

    log::trace!("doing http post request...");
    let response = request.submit().context("do post request")?;
    handle_response(response)?;

    Ok(())
}

fn read_body(mut response: Response<&mut EspHttpConnection>) -> anyhow::Result<()> {
    let mut buf = [0u8; 128];
    let (_, mut body) = response.split();
    let bytes_read = io::try_read_full(&mut body, &mut buf[..]).map_err(|e| e.0)?;

    log::debug!("read {} bytes", bytes_read);
    if bytes_read == 0 {
        return Ok(());
    }

    match std::str::from_utf8(&buf[0..bytes_read]) {
        Ok(body_string) => log::info!(
            "Response body (truncated to {} bytes): {:?}",
            buf.len(),
            body_string
        ),
        Err(e) => log::error!("Error decoding response body: {:?}", e),
    };

    while body.read(&mut buf)? > 0 {}

    Ok(())
}

fn read_sensor<P: gpio::InputPin + gpio::OutputPin>(
    bus: &mut Bus<SensorData>,
    mut pin: PinDriver<'_, P, gpio::InputOutput>,
) {
    thread::sleep(Duration::from_secs(10));
    dht_hal_drv::dht_read(dht_hal_drv::DhtType::DHT22, &mut pin, delay::Ets).ok();
    thread::sleep(Duration::from_millis(500));

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
        if value.is_correct() {
            log::info!("read_sensor: data={}", value);
            bus.broadcast(value);
        } else {
            log::error!("read_sensor: got invalid data={}", value);
        }

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
    if let Err(err) = tm.init() {
        log::error!("could not init tm1637 error={:?}", err);
    }
    log::trace!("clear tm1637...");
    if let Err(err) = tm.clear() {
        log::error!("could not clear tm1637 error={:?}", err);
    }
    log::trace!("set brightness tm1637...");
    if let Err(err) = tm.set_brightness(128) {
        log::error!("could not set brightness tm1637 error={:?}", err);
    }

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

impl SensorData {
    fn is_correct(&self) -> bool {
        let humidity_correct = self.humidity > 0.0 && self.humidity < 100.0;
        let temperature_correct = self.temperature >= -40.0 && self.temperature <= 80.0;

        humidity_correct && temperature_correct
    }
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
    nvs: Option<EspDefaultNvsPartition>,
) -> anyhow::Result<Box<EspWifi<'_>>> {
    let ssid = CONFIG.ssid;
    let pass = CONFIG.password;
    if ssid.is_empty() {
        bail!("Missing WiFi name")
    }
    if pass.is_empty() {
        bail!("Missing WiFi password")
    }

    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), nvs)?;
    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;

    log::info!("Starting wifi...");

    wifi.start()?;

    log::info!("Scanning...");

    let ap_infos = wifi.scan()?;
    for ap in &ap_infos {
        log::info!("found ap {:?}", ap);
    }

    let ours = ap_infos.into_iter().find(|a| a.ssid == ssid);

    let channel = if let Some(ours) = ours {
        log::info!(
            "Found configured access point {} on channel {} with signal strength {}",
            ssid,
            ours.channel,
            ours.signal_strength,
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
