#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

mod error;
mod wifi;

use embassy_executor::Executor;
use embassy_net::Stack;
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Timer};
use error::Error;
use esp_backtrace as _;
use esp_sensor::{http_compat, influx, line_proto};
use esp_wifi::wifi::WifiDevice;
use hal::{
    clock::ClockControl,
    embassy,
    gpio::{GpioPin, OpenDrain, Output},
    peripherals::Peripherals,
    prelude::*,
    timer::TimerGroup,
    Delay, Rtc, IO,
};

macro_rules! singleton {
    ($val:expr) => {{
        use embassy_executor::_export::StaticCell;

        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        let (x,) = STATIC_CELL.init(($val,));
        x
    }};
}
use reqwless::client::HttpClient;
pub(crate) use singleton;

type PubSubChannel = embassy_sync::pubsub::PubSubChannel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    SensorReadings,
    1,
    4,
    4,
>;
type Publisher<'p> = embassy_sync::pubsub::Publisher<
    'p,
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    SensorReadings,
    1,
    4,
    4,
>;
type Subscriber<'s> = embassy_sync::pubsub::Subscriber<
    's,
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    SensorReadings,
    1,
    4,
    4,
>;

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
}

#[global_allocator]
static ALLOCATOR: esp_alloc::EspHeap = esp_alloc::EspHeap::empty();

fn init_heap() {
    const HEAP_SIZE: usize = 32 * 1024;

    extern "C" {
        static mut _heap_start: u32;
        static mut _heap_end: u32;
    }

    unsafe {
        let heap_start = &_heap_start as *const _ as usize;
        let heap_end = &_heap_end as *const _ as usize;
        assert!(
            heap_end - heap_start > HEAP_SIZE,
            "Not enough available heap memory."
        );
        ALLOCATOR.init(heap_start as *mut u8, HEAP_SIZE);
    }
}

#[entry]
fn main() -> ! {
    init_heap();
    esp_println::logger::init_logger(log::LevelFilter::Trace);

    let peripherals = Peripherals::take();
    let mut system = peripherals.DPORT.split();
    let clocks =
        ClockControl::configure(system.clock_control, hal::clock::CpuClock::Clock240MHz).freeze();

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

    embassy::init(&clocks, timer_group0.timer0);
    let (stack, controller) = wifi::init(
        timer_group1.timer0,
        peripherals.RNG,
        peripherals.RADIO,
        system.radio_clock_control,
        &clocks,
    );

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let dht22_pin = io.pins.gpio15.into_open_drain_output();
    let delay: Delay = hal::Delay::new(&clocks);

    let tm = {
        let clk = io.pins.gpio12.into_open_drain_output();
        let dio = io.pins.gpio13.into_open_drain_output();

        let mut tm: tm1637::TM1637<
            GpioPin<Output<OpenDrain>, 12>,
            GpioPin<Output<OpenDrain>, 13>,
            hal::Delay,
        > = tm1637::TM1637::new(clk, dio, delay);
        tm.init().unwrap();
        tm.clear().unwrap();
        tm.set_brightness(128).unwrap();
        tm
    };

    let ps: &'static PubSubChannel = &*singleton!(PubSubChannel::new());

    log::info!("using {:?}", CONFIG);
    log::info!("mac address {:X?}", stack.ethernet_address());

    let executor: &'static mut Executor = singleton!(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(wifi::connection(controller)).unwrap();
        spawner.spawn(wifi::net_task(stack)).unwrap();

        spawner
            .spawn(sensor_reader(dht22_pin, delay, ps.publisher().unwrap()))
            .unwrap();
        spawner
            .spawn(sensor_data_sender(stack, ps.subscriber().unwrap()))
            .unwrap();
        spawner
            .spawn(display_readings(tm, ps.subscriber().unwrap()))
            .unwrap();
    });
}

#[embassy_executor::task]
async fn display_readings(
    mut tm: tm1637::TM1637<
        GpioPin<Output<OpenDrain>, 12>,
        GpioPin<Output<OpenDrain>, 13>,
        hal::Delay,
    >,
    mut subscriber: Subscriber<'static>,
) {
    loop {
        let SensorReadings {
            humidity,
            temperature,
        } = match subscriber.next_message().await {
            WaitResult::Lagged(num) => {
                log::warn!("Lagged {} messages", num);
                continue;
            }
            WaitResult::Message(readings) => readings,
        };

        log::info!("Temperature: {:2.1}°C", temperature);
        log::info!("Humidity:    {:2.1}%", humidity);

        let digits = [
            ((temperature / 10.) as u32 % 10) as u8,
            (temperature as u32 % 10) as u8,
            ((humidity / 10.) as u32 % 10) as u8,
            (humidity as u32 % 10) as u8,
        ];
        if let Err(err) = tm.print_hex(0, &digits) {
            log::error!("could not print hex on tm1637: {:?}", err);
        }
    }
}

#[embassy_executor::task]
async fn sensor_reader(
    mut dht22_pin: GpioPin<Output<OpenDrain>, 15>,
    delay: Delay,
    publisher: Publisher<'static>,
) {
    Timer::after(Duration::from_secs(2)).await;

    loop {
        let value = match dht_hal_drv::dht_read(dht_hal_drv::DhtType::DHT22, &mut dht22_pin, delay)
        {
            Result::Ok(x) => x,
            Result::Err(err) => {
                log::error!("error reading dht sensor: {:?}", err);
                Timer::after(Duration::from_secs(30)).await;
                continue;
            }
        };

        publisher.publish(value.into()).await;

        Timer::after(Duration::from_secs(30)).await;
    }
}

#[embassy_executor::task]
async fn sensor_data_sender(
    stack: &'static Stack<WifiDevice<'static>>,
    mut subscriber: Subscriber<'static>,
) {
    loop {
        let result = sending_loop(stack, &mut subscriber).await;
        if let Err(err) = result {
            log::error!("something went wrong while sending sensor data: {:?}", err);
            Timer::after(Duration::from_secs(10)).await;
        };
    }
}

async fn sending_loop(
    stack: &'static Stack<WifiDevice<'static>>,
    subscriber: &mut Subscriber<'static>,
) -> Result<(), Error> {
    loop {
        if stack.is_link_up() {
            log::info!("Link is up!");
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    log::info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            log::info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    let connector = http_compat::TcpConnect::<WifiDevice<'static>>::new(stack);
    let dns = http_compat::Dns::new(stack);
    let mut client = HttpClient::new(&connector, &dns);

    log::info!("connecting...");
    let resource = client.resource(CONFIG.addr).await?;
    let mut client = influx::Client::new(resource, CONFIG.influx_token);
    log::info!("connected!");

    loop {
        let value = match subscriber.next_message().await {
            WaitResult::Lagged(num) => {
                log::warn!("Lagged {} messages", num);
                continue;
            }
            WaitResult::Message(readings) => readings,
        };

        let mut body = [0; 1024];
        let n_left = {
            let unwritten_part = line_proto::new(&mut body[..])
                .measurement("dht22")?
                .next()?
                .field("humidity", value.humidity)?
                .field("temperature", value.temperature)?
                .next()
                .build()?;
            unwritten_part.len()
        };
        let body = &body[..body.len() - n_left];
        log::debug!("http request body is\n{:?}", core::str::from_utf8(body));

        log::debug!("sending http request...");
        let result = client
            .write(CONFIG.influx_org, CONFIG.influx_bucket, body)
            .await;
        log::debug!("received http response: {:?}", result);

        if result.is_err() {
            log::error!("influxdb API request failed");
        }
    }
}

#[derive(Clone, Copy)]
struct SensorReadings {
    temperature: f32,
    humidity: f32,
}

impl From<dht_hal_drv::DhtValue> for SensorReadings {
    fn from(value: dht_hal_drv::DhtValue) -> Self {
        Self {
            temperature: value.temperature(),
            humidity: value.humidity(),
        }
    }
}
