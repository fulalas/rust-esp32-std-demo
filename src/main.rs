#![allow(unused_imports)]
#![allow(clippy::single_component_path_imports)]

#[cfg(all(feature = "qemu", not(esp32)))]
compile_error!("The `qemu` feature can only be built for the `xtensa-esp32-espidf` target.");

#[cfg(all(feature = "ip101", not(esp32)))]
compile_error!("The `ip101` feature can only be built for the `xtensa-esp32-espidf` target.");

#[cfg(all(feature = "esp32s3_usb_otg", not(esp32s3)))]
compile_error!(
    "The `esp32s3_usb_otg` feature can only be built for the `xtensa-esp32s3-espidf` target."
);

use core::ffi;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::{cell::RefCell, env, sync::atomic::*, sync::Arc, thread, time::*};

use anyhow::{bail, Result};
use log::*;
use url;
use smol;

use embedded_hal::adc::OneShot;
use embedded_hal::blocking::delay::DelayMs;
use embedded_hal::digital::v2::OutputPin;

use embedded_svc::eth;
use embedded_svc::io;
use embedded_svc::ipv4;
use embedded_svc::mqtt::client::{Client, Connection, MessageImpl, Publish, QoS};
use embedded_svc::ping::Ping;
use embedded_svc::sys_time::SystemTime;
use embedded_svc::timer::TimerService;
use embedded_svc::timer::*;
use embedded_svc::utils::mqtt::client::ConnState;
use embedded_svc::wifi::*;

use esp_idf_svc::eventloop::*;
use esp_idf_svc::httpd as idf;
use esp_idf_svc::httpd::ServerRegistry;
use esp_idf_svc::mqtt::client::*;
use esp_idf_svc::netif::*;
use esp_idf_svc::nvs::*;
use esp_idf_svc::ping;
use esp_idf_svc::sntp;
use esp_idf_svc::systime::EspSystemTime;
use esp_idf_svc::timer::*;
use esp_idf_svc::wifi::*;

use esp_idf_hal::adc;
use esp_idf_hal::delay;
use esp_idf_hal::gpio;
use esp_idf_hal::i2c;
use esp_idf_hal::peripheral;
use esp_idf_hal::prelude::*;
use esp_idf_hal::spi;

use esp_idf_sys;
use esp_idf_sys::{esp, EspError};

use display_interface_spi::SPIInterfaceNoCS;

use embedded_graphics::mono_font::{ascii::FONT_10X20, MonoTextStyle};
use embedded_graphics::pixelcolor::*;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::*;
use embedded_graphics::text::*;

use mipidsi;
use ssd1306;
use ssd1306::mode::DisplayConfig;

extern crate fatfs;
use crate::url::Host;
use std::fs::File;

use epd_waveshare::{epd4in2::*, graphics::VarDisplay, prelude::*};

#[allow(dead_code)]
#[cfg(not(feature = "qemu"))]
const SSID: &str = env!("RUST_ESP32_STD_DEMO_WIFI_SSID");
#[allow(dead_code)]
#[cfg(not(feature = "qemu"))]
const PASS: &str = env!("RUST_ESP32_STD_DEMO_WIFI_PASS");

#[cfg(esp32s2)]
include!(env!("EMBUILD_GENERATED_SYMBOLS_FILE"));

#[cfg(esp32s2)]
const ULP: &[u8] = include_bytes!(env!("EMBUILD_GENERATED_BIN_FILE"));

thread_local! {
    static TLS: RefCell<u32> = RefCell::new(13);
}

static CS: esp_idf_hal::task::CriticalSection = esp_idf_hal::task::CriticalSection::new();

fn main() -> Result<()> {
    esp_idf_sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    #[allow(unused)]
    let peripherals = Peripherals::take().unwrap();
    #[allow(unused)]
    let pins = peripherals.pins;

    {
        info!("Testing critical sections");

        {
            let th = {
                let _guard = CS.enter();

                let th = std::thread::spawn(move || {
                    info!("Waiting for critical section");
                    let _guard = CS.enter();

                    info!("Critical section acquired");
                });

                std::thread::sleep(Duration::from_secs(5));

                th
            };

            th.join().unwrap();
        }
    }

    #[allow(unused)]
    let sysloop = EspSystemEventLoop::take()?;
    
    #[allow(clippy::redundant_clone)]
    #[cfg(not(feature = "qemu"))]
    #[allow(unused_mut)]
    let mut wifi = wifi(
        peripherals.modem,
        sysloop.clone(),
        pins.gpio9,
        pins.gpio4,
        pins.gpio8,
        peripherals.spi3,
        pins.gpio6,
        pins.gpio7,
        pins.gpio5
        )?;

    #[allow(clippy::redundant_clone)]
    #[cfg(feature = "qemu")]
    let eth = {
        let mut eth = Box::new(esp_idf_svc::eth::EspEth::wrap(
            esp_idf_svc::eth::EthDriver::new_openeth(peripherals.mac, sysloop.clone()),
        ))?;
        eth_configure(&sysloop, &mut eth)?;

        eth
    };

    #[allow(clippy::redundant_clone)]
    #[cfg(feature = "ip101")]
    let eth = {
        let mut eth = Box::new(esp_idf_svc::eth::EspEth::wrap(
            esp_idf_svc::eth::EthDriver::new_rmii(
                peripherals.mac,
                pins.gpio25,
                pins.gpio26,
                pins.gpio27,
                pins.gpio23,
                pins.gpio22,
                pins.gpio21,
                pins.gpio19,
                pins.gpio18,
                esp_idf_svc::eth::RmiiClockConfig::<gpio::Gpio0, gpio::Gpio16, gpio::Gpio17>::Input(
                    pins.gpio0,
                ),
                Some(pins.gpio5),
                esp_idf_svc::eth::RmiiEthChipset::IP101,
                None,
                sysloop.clone(),
            )?,
        )?);
        eth_configure(&sysloop, &mut eth)?;

        eth
    };

    #[cfg(feature = "w5500")]
    let eth = {
        let mut eth = Box::new(esp_idf_svc::eth::EspEth::wrap(
            esp_idf_svc::eth::EthDriver::new_spi(
                spi::SpiDriver::new(
                    peripherals.spi2,
                    pins.gpio13,
                    pins.gpio12,
                    Some(pins.gpio26),
                    &spi::SpiDriverConfig::new().dma(spi::Dma::Auto(4096)),
                )?,
                pins.gpio27,
                Some(pins.gpio14),
                Some(pins.gpio25),
                esp_idf_svc::eth::SpiEthChipset::W5500,
                20.MHz().into(),
                Some(&[0x02, 0x00, 0x00, 0x12, 0x34, 0x56]),
                None,
                sysloop.clone(),
            )?,
        )?);

        eth_configure(&sysloop, &mut eth)?;

        eth
    };

    let _sntp = sntp::EspSntp::new_default()?;
    info!("SNTP initialized");

    #[cfg(not(feature = "qemu"))]
    #[cfg(esp_idf_lwip_ipv4_napt)]
    enable_napt(&mut wifi)?;

    let mutex = Arc::new((Mutex::new(None), Condvar::new()));

    let httpd = httpd(mutex.clone())?;

    #[cfg(feature = "ssd1306g")]
    {
        for s in 0..3 {
            info!("Powering off the display in {} secs", 3 - s);
            thread::sleep(Duration::from_secs(1));
        }

        led_power.set_low()?;
    }

    let mut wait = mutex.0.lock().unwrap();

    #[cfg(all(esp32, esp_idf_version_major = "4"))]
    let mut hall_sensor = peripherals.hall_sensor;

    #[cfg(esp32)]
    let adc_pin = pins.gpio34;
    #[cfg(not(esp32))]
    let adc_pin = pins.gpio2;

    let mut a2 = adc::AdcChannelDriver::<_, adc::Atten11dB<adc::ADC1>>::new(adc_pin)?;

    let mut powered_adc1 = adc::AdcDriver::new(
        peripherals.adc1,
        &adc::config::Config::new().calibration(true),
    )?;

    #[allow(unused)]
    let cycles = loop {
        if let Some(cycles) = *wait {
            break cycles;
        } else {
            wait = mutex
                .1
                .wait_timeout(wait, Duration::from_secs(1))
                .unwrap()
                .0;

            #[cfg(all(esp32, esp_idf_version_major = "4"))]
            log::info!(
                "Hall sensor reading: {}mV",
                powered_adc1.read_hall(&mut hall_sensor).unwrap()
            );
            log::info!(
                "A2 sensor reading: {}mV",
                powered_adc1.read(&mut a2).unwrap()
            );
        }
    };

    for s in 0..3 {
        info!("Shutting down in {} secs", 3 - s);
        thread::sleep(Duration::from_secs(1));
    }

    drop(httpd);
    info!("Httpd stopped");

    #[cfg(not(feature = "qemu"))]
    {
        drop(wifi);
        info!("Wifi stopped");
    }

    #[cfg(any(feature = "qemu", feature = "w5500", feature = "ip101"))]
    {
        drop(eth);
        info!("Eth stopped");
    }

    Ok(())
}

#[derive(Copy, Clone, Debug)]
struct EventLoopMessage(Duration);

impl EspTypedEventSource for EventLoopMessage {
    fn source() -> *const ffi::c_char {
        b"DEMO-SERVICE\0".as_ptr() as *const _
    }
}

impl EspTypedEventSerializer<EventLoopMessage> for EventLoopMessage {
    fn serialize<R>(
        event: &EventLoopMessage,
        f: impl for<'a> FnOnce(&'a EspEventPostData) -> R,
    ) -> R {
        f(&unsafe { EspEventPostData::new(Self::source(), Self::event_id(), event) })
    }
}

impl EspTypedEventDeserializer<EventLoopMessage> for EventLoopMessage {
    fn deserialize<R>(
        data: &EspEventFetchData,
        f: &mut impl for<'a> FnMut(&'a EventLoopMessage) -> R,
    ) -> R {
        f(unsafe { data.as_payload() })
    }
}

#[cfg(feature = "esp32s3_usb_otg")]
fn esp32s3_usb_otg_hello_world(
    backlight: gpio::Gpio9,
    dc: gpio::Gpio4,
    rst: gpio::Gpio8,
    spi: spi::SPI3,
    sclk: gpio::Gpio6,
    sdo: gpio::Gpio7,
    cs: gpio::Gpio5,
    screen_text: &str,
) -> Result<()> {
    info!("About to initialize the ESP32-S3-USB-OTG SPI LED driver ST7789VW");

    let mut backlight = gpio::PinDriver::output(backlight)?;
    backlight.set_high()?;

    let di = SPIInterfaceNoCS::new(
        spi::SpiDeviceDriver::new_single(
            spi,
            sclk,
            sdo,
            Option::<gpio::AnyIOPin>::None,
            Some(cs),
            &spi::SpiDriverConfig::new().dma(spi::Dma::Disabled),
            &spi::SpiConfig::new().baudrate(80.MHz().into()),
        )?,
        gpio::PinDriver::output(dc)?,
    );

    let mut display = mipidsi::Builder::st7789(di)
        .init(&mut delay::Ets, Some(gpio::PinDriver::output(rst)?))
        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    display
        .set_orientation(mipidsi::options::Orientation::Landscape(true))
        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    led_draw(&mut display, screen_text).map_err(|e| anyhow::anyhow!("Led draw error: {:?}", e))
}

#[allow(dead_code)]
fn led_draw<D>(display: &mut D, screen_text: &str) -> Result<(), D::Error>
where
    D: DrawTarget + Dimensions,
    D::Color: RgbColor,
{
    display.clear(RgbColor::BLACK)?;

    Rectangle::new(display.bounding_box().top_left, display.bounding_box().size)
        .into_styled(
            PrimitiveStyleBuilder::new()
                .fill_color(RgbColor::BLUE)
                .stroke_color(RgbColor::YELLOW)
                .stroke_width(1)
                .build(),
        )
        .draw(display)?;

    Text::new(
        screen_text,
        Point::new(10, (display.bounding_box().size.height - 10) as i32 / 2),
        MonoTextStyle::new(&FONT_10X20, RgbColor::WHITE),
    )
    .draw(display)?;

    info!("LED rendering done");

    Ok(())
}

#[allow(unused_variables)]
fn httpd(
    mutex: Arc<(Mutex<Option<u32>>, Condvar)>,
) -> Result<esp_idf_svc::http::server::EspHttpServer> {
    use embedded_svc::http::server::{
        Connection, Handler, HandlerResult, Method, Middleware, Query, Request, Response,
    };
    use embedded_svc::io::Write;
    use esp_idf_svc::http::server::{fn_handler, EspHttpConnection, EspHttpServer};

    struct SampleMiddleware {}

    impl<C> Middleware<C> for SampleMiddleware
    where
        C: Connection,
    {
        fn handle<'a, H>(&'a self, connection: &'a mut C, handler: &'a H) -> HandlerResult
        where
            H: Handler<C>,
        {
            let req = Request::wrap(connection);

            info!("Middleware called with uri: {}", req.uri());

            let connection = req.release();

            if let Err(err) = handler.handle(connection) {
                if !connection.is_response_initiated() {
                    let mut resp = Request::wrap(connection).into_status_response(500)?;

                    write!(&mut resp, "ERROR: {err}")?;
                } else {
                    // Nothing can be done as the error happened after the response was initiated, propagate further
                    return Err(err);
                }
            }

            Ok(())
        }
    }

    struct SampleMiddleware2 {}

    impl<C> Middleware<C> for SampleMiddleware2
    where
        C: Connection,
    {
        fn handle<'a, H>(&'a self, connection: &'a mut C, handler: &'a H) -> HandlerResult
        where
            H: Handler<C>,
        {
            info!("Middleware2 called");

            handler.handle(connection)
        }
    }

    let mut server = EspHttpServer::new(&Default::default())?;

    server
        .fn_handler("/", Method::Get, |req| {
            req.into_ok_response()?
                .write_all("mSupply FTW!".as_bytes())?;

            Ok(())
        })?
        .fn_handler("/bar", Method::Get, |req| {
            req.into_response(403, Some("No permissions"), &[])?
                .write_all("You have no permissions to access this page".as_bytes())?;

            Ok(())
        })?;

    Ok(server)
}

#[cfg(not(feature = "qemu"))]
#[allow(dead_code)]
fn wifi(
    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
    backlight: gpio::Gpio9,
    dc: gpio::Gpio4,
    rst: gpio::Gpio8,
    spi: spi::SPI3,
    sclk: gpio::Gpio6,
    sdo: gpio::Gpio7,
    cs: gpio::Gpio5,
) -> Result<Box<EspWifi<'static>>> {
    use std::net::Ipv4Addr;

    use esp_idf_svc::handle::RawHandle;

    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;

    info!("Starting wifi...");

    wifi.start()?;

    info!("Scanning...");

    let ap_infos = wifi.scan()?;

    let ours = ap_infos.into_iter().find(|a| a.ssid == SSID);

    let channel = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {}",
            SSID, ours.channel
        );
        Some(ours.channel)
    } else {
        info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            SSID
        );
        None
    };

    wifi.set_configuration(&Configuration::Mixed(
        ClientConfiguration {
            ssid: SSID.into(),
            password: PASS.into(),
            channel,
            ..Default::default()
        },
        AccessPointConfiguration {
            ssid: "aptest".into(),
            channel: channel.unwrap_or(1),
            ..Default::default()
        },
    ))?;

    info!("Connecting wifi...");

    wifi.connect()?;

    info!("Waiting for DHCP lease...");

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    esp32s3_usb_otg_hello_world(
        backlight,
        dc,
        rst,
        spi,
        sclk,
        sdo,
        cs,
        format!("Powered by mSupply\n\nIP: {:?}", ip_info.ip.to_string().as_str()).as_str()
    )?;

    info!("Wifi DHCP info: {:?}", ip_info);

    ping(ip_info.subnet.gateway)?;

    Ok(Box::new(esp_wifi))
}

#[cfg(any(feature = "qemu", feature = "w5500", feature = "ip101"))]
fn eth_configure<'d, T>(
    sysloop: &EspSystemEventLoop,
    eth: &mut esp_idf_svc::eth::EspEth<'d, T>,
) -> Result<()> {
    use std::net::Ipv4Addr;

    info!("Eth created");

    let mut eth = esp_idf_svc::eth::BlockingEth::wrap(eth, sysloop.clone())?;

    info!("Starting eth...");

    eth.start()?;

    info!("Waiting for DHCP lease...");

    eth.wait_netif_up()?;

    let ip_info = eth.eth().netif().get_ip_info()?;

    info!("Eth DHCP info: {:?}", ip_info);

    ping(ip_info.subnet.gateway)?;

    Ok(())
}

fn ping(ip: ipv4::Ipv4Addr) -> Result<()> {
    info!("About to do some pings for {:?}", ip);

    let ping_summary = ping::EspPing::default().ping(ip, &Default::default())?;
    if ping_summary.transmitted != ping_summary.received {
        bail!("Pinging IP {} resulted in timeouts", ip);
    }

    info!("Pinging done");

    Ok(())
}

#[cfg(not(feature = "qemu"))]
#[cfg(esp_idf_lwip_ipv4_napt)]
fn enable_napt(wifi: &mut EspWifi) -> Result<()> {
    wifi.ap_netif_mut().enable_napt(true);

    info!("NAPT enabled on the WiFi SoftAP!");

    Ok(())
}
