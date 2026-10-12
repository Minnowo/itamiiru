//! xdg-desktop-portal-itamiiru: the D-Bus FileChooser backend. Linux only.
//!
//! The portal frontend calls us on the session bus; for each call we run
//! `itami --pick` and reply with its answer.
//! With `--file-manager` it serves org.freedesktop.FileManager1 instead
//! ("Show in folder"), as a separate process.

#[cfg(target_os = "linux")]
#[cfg(target_os = "linux")]
mod filemanager;
#[cfg(target_os = "linux")]
mod portal;

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::mpsc;

    use filemanager::FileManager;
    use itamiiru_core::platform::dbus::conn::{self, Connection};
    use portal::{Event, Portal};

    let file_manager = std::env::args().skip(1).any(|arg| arg == "--file-manager");
    let bus_name = if file_manager {
        filemanager::BUS_NAME
    } else {
        portal::BUS_NAME
    };

    let Some(address) = conn::session_address() else {
        eprintln!("xdg-desktop-portal-itamiiru: no session bus address");
        std::process::exit(1);
    };
    let (mut connection, mut reader) = match Connection::open(&address) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("xdg-desktop-portal-itamiiru: can't connect to the session bus: {err}");
            std::process::exit(1);
        }
    };
    if let Err(err) = connection.register(&mut reader, bus_name) {
        eprintln!("xdg-desktop-portal-itamiiru: {err}");
        std::process::exit(1);
    }
    eprintln!(
        "xdg-desktop-portal-itamiiru: ready as {} ({})",
        bus_name, connection.unique_name
    );

    // The reader thread turns socket data into events; picker threads add
    // PickDone events. The main thread owns the writing half.
    let (tx, rx) = mpsc::channel();
    let reader_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            match conn::read_message(&mut reader) {
                Ok(msg) => {
                    if reader_tx.send(Event::Incoming(msg)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = reader_tx.send(Event::Disconnected(err.to_string()));
                    return;
                }
            }
        }
    });

    if file_manager {
        let mut file_manager = FileManager::new(connection);
        for event in rx.iter() {
            if !file_manager.handle(event) {
                break;
            }
        }
        return;
    }
    let mut portal = Portal::new(connection, tx);
    for event in rx.iter() {
        if !portal.handle(event) {
            break;
        }
    }
    portal.shutdown();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("xdg-desktop-portal-itamiiru only runs on Linux");
    std::process::exit(1);
}
