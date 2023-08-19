use super::singleton;
use embassy_net::{Config, Stack, StackResources};
use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_wifi::{
    initialize,
    wifi::{WifiController, WifiDevice, WifiEvent, WifiMode, WifiState},
    EspWifiInitFor,
};
use hal::{
    self,
    clock::Clocks,
    peripherals::{self, TIMG1},
    radio::RadioExt,
    system::RadioClockControl,
    timer::Timer0,
    Timer as HalTimer,
};

#[inline(always)]
pub(crate) fn init<'d>(
    timer0: HalTimer<Timer0<TIMG1>>,
    rng: peripherals::RNG,
    radio: peripherals::RADIO,
    radio_clock_control: RadioClockControl,
    clocks: &Clocks,
) -> (&'static mut Stack<WifiDevice<'d>>, WifiController<'d>) {
    let mut rng = hal::Rng::new(rng);
    let seed = rng.random();
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer0,
        rng,
        radio_clock_control,
        clocks,
    )
    .unwrap();

    let (wifi, _) = radio.split();
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiMode::Sta).unwrap();
    let config = Config::dhcpv4(Default::default());

    log::debug!("wifi::init: seed={}", seed);
    let seed = seed as u64;

    let stack = singleton!(Stack::new(
        wifi_interface,
        config,
        singleton!(StackResources::<2>::new()),
        seed
    ));

    (stack, controller)
}

#[embassy_executor::task]
pub(crate) async fn connection(mut controller: WifiController<'static>) {
    log::info!("connection: start connection task");
    log::info!(
        "connection: device capabilities={:?}",
        controller.get_capabilities()
    );
    loop {
        if let WifiState::StaConnected = esp_wifi::wifi::get_wifi_state() {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            log::trace!("connection: sleeping for 5s...");
            Timer::after(Duration::from_secs(5)).await
        }

        match controller.is_started() {
            Ok(false) => {
                let client_config = Configuration::Client(ClientConfiguration {
                    ssid: super::CONFIG.ssid.into(),
                    password: super::CONFIG.password.into(),
                    ..Default::default()
                });
                controller.set_configuration(&client_config).unwrap();
                log::trace!("starting wifi...");
                controller.start().await.unwrap();
                log::trace!("wifi started!");
            }
            Ok(true) => {
                log::trace!("connection: stopping wifi...");
                if let Err(err) = controller.stop().await {
                    log::error!("connection: failed to stop wifi error={:?}", err);
                }
                continue;
            }
            Err(err) => {
                log::error!(
                    "connection: failed to check if wifi is started error={:?}",
                    err
                );
            }
        };

        log::debug!("about to connect...");
        match controller.connect().await {
            Ok(_) => log::info!("wifi connected!"),
            Err(e) => {
                log::error!("failed to connect to wifi error={e:?}");
                log::trace!("connection: sleeping for 10s...");
                Timer::after(Duration::from_secs(10)).await
            }
        }
    }
}

#[embassy_executor::task]
pub(crate) async fn net_task(stack: &'static Stack<WifiDevice<'static>>) {
    stack.run().await
}
