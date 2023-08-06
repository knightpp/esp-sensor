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
    clock::Clocks,
    peripherals::{self, TIMG1},
    radio::RadioExt,
    rng::Rng as HalRng,
    system::RadioClockControl,
    timer::Timer0,
    Timer as HalTimer,
};

pub(crate) fn init<'d>(
    timer0: HalTimer<Timer0<TIMG1>>,
    rng: peripherals::RNG,
    radio: peripherals::RADIO,
    radio_clock_control: RadioClockControl,
    clocks: &Clocks,
) -> (&'static mut Stack<WifiDevice<'d>>, WifiController<'d>) {
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer0,
        HalRng::new(rng),
        radio_clock_control,
        clocks,
    )
    .unwrap();

    let (wifi, _) = radio.split();
    let (wifi_interface, controller) = esp_wifi::wifi::new_with_mode(&init, wifi, WifiMode::Sta);
    let config = Config::dhcpv4(Default::default());

    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = singleton!(Stack::new(
        wifi_interface,
        config,
        singleton!(StackResources::<3>::new()),
        seed
    ));

    (stack, controller)
}

#[embassy_executor::task]
pub(crate) async fn connection(mut controller: WifiController<'static>) {
    log::info!("start connection task");
    log::info!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        if let WifiState::StaConnected = esp_wifi::wifi::get_wifi_state() {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: super::CONFIG.ssid.into(),
                password: super::CONFIG.password.into(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            log::info!("Starting wifi");
            controller.start().await.unwrap();
            log::info!("Wifi started!");
        }
        log::debug!("About to connect...");

        match controller.connect().await {
            Ok(_) => log::info!("Wifi connected!"),
            Err(e) => {
                log::error!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
pub(crate) async fn net_task(stack: &'static Stack<WifiDevice<'static>>) {
    stack.run().await
}
