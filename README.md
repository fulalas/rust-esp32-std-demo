# Rust on ESP32 STD demo app

A demo STD binary crate for the ESP32[XX] and ESP-IDF, which connects to WiFi, Ethernet, drives a small HTTP server and shows the endpoint on the device screen.

# Hardware setup
.The USB DEV port has to be used to power any USB stick connected to the USB HOST port. In the real world though, when plugging it into the fridge, the requirement specifies that the fridge will have enough power to work, so the USB DEV port won't be needed.

.The USB-UART0 port (mini-USB) is used to flash the application into the ESP32-S3.

# Mass storage example (C programming language)
https://github.com/espressif/esp-idf/tree/master/examples/peripherals/usb/host/msc

This application reads a file from the USB stick connected to the USB HOST port and print the result in the developer machine terminal.

	***Install esp-idf***
	git clone --recursive https://github.com/espressif/esp-idf.git
	
	cd esp-idf
	
	./install.sh esp32s3
	
	. ./export.sh

	***Set target***
	Go to the solution folder:
	idf.py set-target esp32s3

	***Detect PORT***
	The device which doesn't appear in the second run is the one we want:
	ls /dev/tty*
	unplug it:
	ls /dev/tty*

	Now we need permission to access the device:
	sudo -S chown -R guest:users /dev/ttyUSB0

	***Build***
	idf.py -p /dev/ttyUSB0 flash monitor

# Access point example rust-esp32-std-demo (Rust programming language)
https://github.com/ivmarkov/rust-esp32-std-demo

	***Configure Rust Environment***
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
	source "$HOME/.cargo/env"
	
	***Build espup (https://github.com/esp-rs/rust-build)***
	Linux x86_64:
		curl -L https://github.com/esp-rs/espup/releases/latest/download/espup-x86_64-unknown-linux-gnu -o espup
	MacOS x86_64:
		curl -L https://github.com/esp-rs/espup/releases/latest/download/espup-x86_64-apple-darwin -o espup
	MacOS aarch64:
		curl -L https://github.com/esp-rs/espup/releases/latest/download/espup-aarch64-apple-darwin -o espup
	
	chmod a+x espup
	./espup install
	. ~/export-esp.sh
	
	***Set default rust for esp***
	rustup default esp
	
	***Install ldproxy***
	cargo install ldproxy
	
	***Clone project (original: https://github.com/ivmarkov/rust-esp32-std-demo)***
	git clone https://github.com/fulalas/rust-esp32-std-demo
	
	cd rust-esp32-std-demo
	
	***Export wifi credentials variables***
    export RUST_ESP32_STD_DEMO_WIFI_SSID=<ssid>
    #export RUST_ESP32_STD_DEMO_WIFI_SSID=Trustpower_2.4GHz_0117
    
    export RUST_ESP32_STD_DEMO_WIFI_PASS=<password>
    #export RUST_ESP32_STD_DEMO_WIFI_PASS=qocuditafa

	[Install Python pip]
	
	***Build for ESP32-S3 -- the flag '--features esp32s3_usb_otg' is to enable the LCD***
	cargo build --release --target xtensa-esp32s3-espidf --features esp32s3_usb_otg
	
	In case of this error:
		error[E0609]: no field `tm_gmtoff` on type `tm`
		   --> ~/.cargo/registry/src/index.crates.io-6f17d22bba15001f/time-0.1.45/src/sys.rs:394:30
				|
			394 |             let gmtoff = out.tm_gmtoff;
				|                              ^^^^^^^^^ unknown field

	Solution is changing the offending line to:
	let gmtoff = out.tm_hour;
	
	***Install espflash***
	cargo install espflash
	
	***Detect PORT***
	The device which doesn't appear in the second run is the one we want:
	ls /dev/tty*
	unplug it:
	ls /dev/tty*

	Now we need permission to access the device (considering it's ttyUSB0):
	sudo -S chown -R guest:users /dev/ttyUSB0
	
	***Flash application into ESP32-S3***
	#espflash /dev/ttyUSB0 target/xtensa-esp32s3-espidf/release/rust-esp32-std-demo
	espflash flash target/xtensa-esp32s3-espidf/release/rust-esp32-std-demo

	The device will reboot and run the application. After a while it should connect to the wifi network and display its IP address that will be the endpoint.

	***Get the application output in the developer machine***
	espflash serial-monitor
	
	It will probably print something like this:
		Detected 2 serial ports. Ports which match a known common dev board are highlighted.

		  /dev/ttyACM0 - USB_JTAG_serial_debug_unit
		❯ /dev/ttyUSB0 - CP2102N_USB_to_UART_Bridge_Controller
	
	Select with the keyboard arrow key the device used to flash the application -- in my case /dev/ttyUSB0 -- then press Enter.
	
	If you want to change the code, you need to build and flash.

# Read ESP32-S3 internal storage (Rust programming language)
https://github.com/esp-rs/esp-storage

This project might be useful when configuring the board for a given wifi network. One idea is to plug a USB stick on the board containing a plain text file with the wifi ssid and password and the board would copy this file into its internal storage so when plugging the board into the fridge it would use this network information to connect and provide and IP for its endpoints.

After spending a couple of hours putting this project to run, I realized it is too low level and doesn't seem to help much in our goal.

# Troubleshooting

Error: espflash::connection_failed
Solution: edit '[project_folder]/.cargo/config.toml' and change the esp32-s3 runner line from 'runner = "espflash flash --monitor"' to 'runner = "espflash --monitor"'

Error: error: linker `xtensa-esp32s3-elf-gcc` not found
Solution: need to install the Rust environment using the default method, not the minimal

# Useful docs
https://github.com/espressif/esp-idf/issues/11481<br />
https://github.com/nviennot/tinyusb-sys-rs<br />
https://github.com/esp-rs/awesome-esp-rust<br />

https://hackaday.com/2021/03/26/usb-comes-to-the-esp32/<br />
https://github.com/espressif/esp-dev-kits/blob/master/docs/en/esp32s3/esp32-s3-usb-otg/user_guide.rst<br />
https://wokwi.com/projects/322410731508073042<br />
