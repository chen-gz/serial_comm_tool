// #![windows_subsystem = "windows"]

use slint::{Model, ModelRc, SharedString, VecModel, Weak};
slint::include_modules!();
use serialport::{available_ports, DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::{
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

#[derive(Debug)]
enum CmdToSerial {
    Open(String, u32), // port name, baudrate
    Send(Vec<u8>),
    Close,
}

#[derive(Debug)]
enum CmdToUI {
    UpdateRecvData(Vec<u8>),
    SendDataSuccess(u32),
    PortClosed,
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let (tx_to_serial, rx_to_serial) = mpsc::channel();
    let (tx_to_ui, rx_to_ui) = mpsc::channel();

    setup_serial_thread(rx_to_serial, tx_to_ui.clone());
    setup_ui_update_thread(rx_to_ui, ui.as_weak(), tx_to_serial.clone());
    setup_ui_event_listeners(&ui, tx_to_serial);

    ui.run()
}

fn setup_serial_thread(rx_to_serial: Receiver<CmdToSerial>, tx_to_ui: Sender<CmdToUI>) {
    thread::spawn(move || {
        let mut port: Option<Box<dyn SerialPort>> = None;
        let mut tx_close: Sender<()> = mpsc::channel().0; // Dummy initialization
        loop {
            match rx_to_serial.recv() {
                Ok(cmd) => match cmd {
                    CmdToSerial::Open(port_name, baudrate) => {
                        println!("Open port: {}, {}", port_name, baudrate);
                        if port.is_some() {
                            let _ = tx_close.send(()); // Attempt to close any existing port
                            port = None; // Drop the existing port
                        }
                        match serialport::new(&port_name, baudrate)
                            .data_bits(DataBits::Eight)
                            .flow_control(FlowControl::None)
                            .parity(Parity::None)
                            .stop_bits(StopBits::One)
                            .timeout(Duration::from_millis(100))
                            .open()
                        {
                            Ok(p) => {
                                port = Some(p);
                                let port_clone = port
                                    .as_ref()
                                    .unwrap()
                                    .try_clone()
                                    .expect("Failed to clone port");
                                let tx_clone = tx_to_ui.clone();
                                let (tx, rx) = mpsc::channel();
                                tx_close = tx;
                                thread::spawn(move || {
                                    read_serial_data(port_clone, tx_clone, rx);
                                });
                            }
                            Err(e) => println!("Failed to open serial port: {:?}", e),
                        }
                    }
                    CmdToSerial::Close => {
                        let _ = tx_close.send(()); // Send close signal
                        port = None; // Drop the port
                        println!("Close port in main thread");
                    }
                    CmdToSerial::Send(data) => {
                        if let Some(p) = port.as_mut() {
                            let _ = p.write(&data); // Send data
                            println!("Send data: {:?}", data);
                            tx_to_ui
                                .send(CmdToUI::SendDataSuccess(data.len() as u32))
                                .unwrap();
                        }
                    }
                },
                Err(e) => {
                    println!("Error receiving command: {:?}", e);
                    break;
                }
            }
        }
    });
}

fn read_serial_data(
    mut port: Box<dyn SerialPort>,
    tx_to_ui: Sender<CmdToUI>,
    rx_close: Receiver<()>,
) {
    let mut serial_buf: Vec<u8> = vec![0; 100];
    loop {
        match port.bytes_to_read() {
            Ok(t) if t > 0 => match port.read(serial_buf.as_mut_slice()) {
                Ok(t) => {
                    let s = String::from_utf8_lossy(&serial_buf[..t]);
                    let _ = tx_to_ui.send(CmdToUI::UpdateRecvData(serial_buf[..t].to_vec()));
                }
                Err(e) => {
                    let _ = tx_to_ui.send(CmdToUI::PortClosed);
                    println!("Error reading from serial port: {:?}", e);
                    break;
                }
            },
            Ok(_) => {}
            Err(e) => {
                let _ = tx_to_ui.send(CmdToUI::PortClosed);
                println!("Error reading from serial port: {:?}", e);
                break;
            }
        }
        if rx_close.try_recv().is_ok() {
            println!("Close port in read thread");
            break;
        }
    }
}

fn setup_ui_update_thread(
    rx_to_ui: Receiver<CmdToUI>,
    ui: Weak<AppWindow>,
    tx_to_serial: Sender<CmdToSerial>,
) {
    // let ui_handle = ui.as_weak();
    thread::spawn(move || {
        loop {
            match rx_to_ui.recv() {
                Ok(cmd) => {
                    println!("UI command: {:?}", cmd);
                    match cmd {
                        CmdToUI::UpdateRecvData(data) => {
                            ui.upgrade_in_event_loop(move |ui| {
                                // append the received data to the existing data
                                let recv_data = ui.get_received_data().to_string();
                                let recv_data = format!("{}{}", recv_data, String::from_utf8_lossy(&data));
                                ui.set_received_data(slint::SharedString::from(recv_data));
                                // update the received data length
                                let recv_len = ui.get_received_data_length() as u32 + data.len() as u32;
                                ui.set_received_data_length(recv_len as i32);
                            }).unwrap();
                        }
                        CmdToUI::PortClosed => {
                            let _ = tx_to_serial.send(CmdToSerial::Close);
                            thread::sleep(Duration::from_millis(100)); // Simulate delay
                            ui.upgrade_in_event_loop(|ui| {
                                ui.set_connect_button_text(slint::SharedString::from("Connect"));
                                ui.invoke_refresh_button_clicked(); // Refresh COM ports list
                            }).unwrap();

                        }
                        CmdToUI::SendDataSuccess(len) => {
                            ui.upgrade_in_event_loop(move |ui| {
                                let sent_len = ui.get_send_data_length() as u32 + len;
                                ui.set_send_data_length(sent_len as i32);
                            })
                            .unwrap();
                        }
                    }
                    // }
                }
                Err(e) => {
                    println!("Error: {:?}", e);
                    break;
                }
            }
        }
    });
}

fn setup_ui_event_listeners(ui: &AppWindow, tx_to_serial: Sender<CmdToSerial>) {
    let tx_to_serial_copy = tx_to_serial.clone();

    ui.on_connect_button_clicked({
        let ui_handle = ui.as_weak();
        move || {
            // if let Some(ui) = ui_handle.upgrade() {
            let ui = ui_handle.upgrade().unwrap();
            let idx = ui.get_com_ports_index();
            let ports = ui.get_com_ports();
            let port_name = ports.row_data(idx as usize).unwrap().to_string();
            let button_text = ui.get_connect_button_text().to_string();
            if button_text == "Connect" {
                let _ = tx_to_serial.send(CmdToSerial::Open(port_name, 115200));
                ui.set_connect_button_text(slint::SharedString::from("Disconnect"));
            } else {
                let _ = tx_to_serial.send(CmdToSerial::Close);
                ui.set_connect_button_text(slint::SharedString::from("Connect"));
            }
            // }
        }
    });

    ui.on_refresh_button_clicked({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let mut ports_name = vec![];
                if let Ok(ports) = available_ports() {
                    for p in ports {
                        ports_name.push(slint::SharedString::from(p.port_name));
                    }
                }
                let the_model = Rc::new(VecModel::from(ports_name));
                let model_rc = ModelRc::new(the_model.clone());
                ui.set_com_ports(model_rc);
            }
        }
    });
    // copy tx_to_serial to the closure
    ui.on_send_button_clicked({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                // get hex check box state
                let hex = ui.get_hex_selected();
                let data = ui.get_send_data().to_string();
                // if hex is selected, convert the string to hex
                if hex {
                    // the data format should be like "1A 2B 2C"
                    let data = data
                        .split_whitespace()
                        .map(|s| u8::from_str_radix(s, 16).unwrap());
                    // convert the string to bytes
                    let data = data.collect::<Vec<u8>>();
                    let _ = tx_to_serial_copy.send(CmdToSerial::Send(data));
                } else {
                    let data = data.as_bytes().to_vec();
                    let _ = tx_to_serial_copy.send(CmdToSerial::Send(data));
                }
            }
        }
    });
}
