#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![forbid(unsafe_code)]

mod wifi;

use dht_hal_drv::DhtValue;
use embassy_executor::Executor;
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_println::println;
use hal::{
    clock::ClockControl,
    embassy,
    gpio::{GpioPin, OpenDrain, Output},
    peripherals::Peripherals,
    prelude::*,
    timer::TimerGroup,
    Rtc, IO,
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

#[toml_cfg::toml_config]
pub struct Config {
    #[default("<CHANGEME>")]
    ssid: &'static str,
    #[default("<CHANGEME>")]
    password: &'static str,
}

#[entry]
fn main() -> ! {
    esp_println::logger::init_logger(log::LevelFilter::Info);

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

    let tm = {
        let clk = singleton!(io.pins.gpio12.into_open_drain_output());
        let dio = singleton!(io.pins.gpio13.into_open_drain_output());
        let delay = singleton!(hal::Delay::new(&clocks));

        let mut tm: tm1637::TM1637<
            'static,
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

    let executor: &'static mut Executor = singleton!(Executor::new());
    executor.run(|spawner| {
        // spawner.spawn(wifi::connection(controller)).unwrap();
        // spawner.spawn(wifi::net_task(stack)).unwrap();

        spawner
            .spawn(sensor_reader(dht22_pin, ps.publisher().unwrap()))
            .unwrap();
        spawner
            .spawn(display_readings(tm, ps.subscriber().unwrap()))
            .unwrap();
    });
}

#[embassy_executor::task]
async fn display_readings(
    mut tm: tm1637::TM1637<
        'static,
        GpioPin<Output<OpenDrain>, 12>,
        GpioPin<Output<OpenDrain>, 13>,
        hal::Delay,
    >,
    mut subscriber: Subscriber<'static>,
) {
    loop {
        let SensorReadings {
            humidity: hum,
            temperature: temp,
        } = match subscriber.next_message().await {
            WaitResult::Lagged(num) => {
                log::warn!("Lagged {} messages", num);
                continue;
            }
            WaitResult::Message(readings) => readings,
        };

        println!("Temperature: {:2.1}°C", temp);
        println!("Humidity:    {:2.1}%", hum);

        let digits = [
            ((temp / 10.) as u32 % 10) as u8,
            (temp as u32 % 10) as u8,
            ((hum / 10.) as u32 % 10) as u8,
            (hum as u32 % 10) as u8,
        ];
        tm.print_hex(0, &digits).unwrap();
    }
}

#[embassy_executor::task]
async fn sensor_reader(
    mut dht22_pin: GpioPin<Output<OpenDrain>, 15>,
    publisher: Publisher<'static>,
) {
    loop {
        let value =
            match dht_hal_drv::dht_read(dht_hal_drv::DhtType::DHT22, &mut dht22_pin, &mut |d| {
                embassy_time::block_for(Duration::from_micros(d as u64));
                // Timer::after(Duration::from_micros(d as u64));
            }) {
                Ok(value) => value,
                Err(err) => {
                    println!("Error: {:?}", err);
                    Timer::after(Duration::from_secs(2)).await;
                    continue;
                }
            };

        publisher.publish(value.into()).await;

        Timer::after(Duration::from_secs(5)).await;
    }
}

#[derive(Clone, Copy)]
struct SensorReadings {
    temperature: f32,
    humidity: f32,
}

impl From<DhtValue> for SensorReadings {
    fn from(value: DhtValue) -> Self {
        Self {
            temperature: value.temperature(),
            humidity: value.humidity(),
        }
    }
}
