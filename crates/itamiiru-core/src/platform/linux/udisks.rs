//! Removable drives through UDisks2, on the system bus: list
//! the filesystems on them, mount one, and eject a drive safely (unmount
//! everything on it, then power it off). UDisks does the actual work as
//! root once polkit allows it; this process never runs `mount` itself.
//! By default polkit allows all of it, without a password, for removable
//! drives in the active local session.
//!
//! Each call opens its own short-lived connection: listing happens every
//! few seconds at most, and mounting is rare.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use super::dbus::conn::{self, Connection};
use super::dbus::msg::{ALLOW_INTERACTIVE_AUTHORIZATION, Message};
use super::dbus::wire::{Type, Value};
use crate::places::Volume;

const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const LOOP: &str = "org.freedesktop.UDisks2.Loop";

/// Listing must answer quickly; it runs every few seconds.
const LIST_TIMEOUT: Duration = Duration::from_secs(5);
/// Mounting can wait on a password prompt, and unmounting on writing out
/// what's still cached for a slow stick.
const ACTION_TIMEOUT: Duration = Duration::from_secs(300);

/// An interface's properties, by name.
type Props = BTreeMap<String, Value>;
/// Every object UDisks has, with its interfaces' properties.
type Objects = BTreeMap<String, BTreeMap<String, Props>>;

/// The filesystems on removable drives, mounted or not. Empty when UDisks
/// isn't running.
pub fn removable_volumes() -> Vec<Volume> {
    return match managed_objects(LIST_TIMEOUT) {
        Ok(objects) => volumes_from_objects(&objects, super::current_uid()),
        Err(err) => {
            crate::debug!("udisks: can't list drives: {err}");
            Vec::new()
        }
    };
}

/// Mounts a volume where UDisks chooses (under /media/<user>/), and
/// returns that folder.
pub fn mount_volume(volume: &Volume) -> Result<PathBuf, String> {
    if let Some(point) = &volume.mount_point {
        return Ok(point.clone());
    }
    let reply = call(
        &volume.id,
        FILESYSTEM,
        "Mount",
        vec![no_options()],
        ACTION_TIMEOUT,
    )?;
    let Some(point) = reply.body.first().and_then(Value::as_str) else {
        return Err("the disk service didn't say where it mounted the drive".to_string());
    };
    return Ok(PathBuf::from(point));
}

/// Unmounts just this filesystem. The drive stays powered and listed, so
/// it can be mounted again without unplugging it.
pub fn unmount_volume(volume: &Volume) -> Result<(), String> {
    call(
        &volume.id,
        FILESYSTEM,
        "Unmount",
        vec![no_options()],
        ACTION_TIMEOUT,
    )?;
    return Ok(());
}

/// Unmounts every filesystem on the volume's drive (or just this one, for
/// a volume without a drive), then powers the drive off or ejects its
/// media; a disk image attached by this user is detached instead. Once the
/// unmounting succeeded, the drive is safe to unplug, so a failed
/// power-off is only logged.
pub fn eject_volume(volume: &Volume) -> Result<(), String> {
    // Fresh state: something else may have mounted more since the last scan.
    let objects = managed_objects(LIST_TIMEOUT).map_err(|err| err.to_string())?;
    for block in blocks_to_unmount(&objects, volume) {
        call(
            &block,
            FILESYSTEM,
            "Unmount",
            vec![no_options()],
            ACTION_TIMEOUT,
        )?;
    }
    if volume.drive_id.is_empty() {
        let is_own_loop = objects
            .get(&volume.id)
            .is_some_and(|interfaces| is_own_loop(interfaces, super::current_uid()));
        if is_own_loop
            && let Err(err) = call(
                &volume.id,
                LOOP,
                "Delete",
                vec![no_options()],
                ACTION_TIMEOUT,
            )
        {
            crate::debug!("udisks: unmounted, but detaching the image failed: {err}");
        }
        return Ok(());
    }
    let Some(drive) = objects
        .get(&volume.drive_id)
        .and_then(|interfaces| interfaces.get(DRIVE))
    else {
        return Ok(());
    };
    let member = if prop_bool(drive, "CanPowerOff") {
        "PowerOff"
    } else if prop_bool(drive, "Ejectable") {
        "Eject"
    } else {
        return Ok(());
    };
    if let Err(err) = call(
        &volume.drive_id,
        DRIVE,
        member,
        vec![no_options()],
        ACTION_TIMEOUT,
    ) {
        crate::debug!("udisks: unmounted, but {member} failed: {err}");
    }
    return Ok(());
}

/// The mounted filesystems that have to be unmounted to eject `volume`:
/// everything on the same drive, or the volume itself when it has none.
fn blocks_to_unmount(objects: &Objects, volume: &Volume) -> Vec<String> {
    let mut out = Vec::new();
    for (path, interfaces) in objects {
        let (Some(block), Some(filesystem)) = (interfaces.get(BLOCK), interfaces.get(FILESYSTEM))
        else {
            continue;
        };
        let same = if volume.drive_id.is_empty() {
            *path == volume.id
        } else {
            prop_str(block, "Drive") == volume.drive_id
        };
        if same && !mount_points(filesystem).is_empty() {
            out.push(path.clone());
        }
    }
    return out;
}

/// Picks the volumes worth showing out of everything UDisks knows: block
/// devices with a filesystem that UDisks itself marks as neither a system
/// disk (internal drives, which need a password to mount) nor one to
/// ignore (swap, recovery partitions). Disk images attached by user `uid`
/// count too, though UDisks calls every loop device a system one.
pub fn volumes_from_objects(objects: &Objects, uid: u32) -> Vec<Volume> {
    let mut volumes = Vec::new();
    for (path, interfaces) in objects {
        let (Some(block), Some(filesystem)) = (interfaces.get(BLOCK), interfaces.get(FILESYSTEM))
        else {
            continue;
        };
        if prop_bool(block, "HintIgnore") {
            continue;
        }
        if prop_bool(block, "HintSystem") && !is_own_loop(interfaces, uid) {
            continue;
        }
        let drive_path = prop_str(block, "Drive");
        // "/" means no drive (a loop device, for example).
        let drive_id = if drive_path == "/" { "" } else { drive_path };
        let drive = objects.get(drive_id).and_then(|i| i.get(DRIVE));
        let can_power_off = match drive {
            Some(drive) => prop_bool(drive, "CanPowerOff") || prop_bool(drive, "Ejectable"),
            None => false,
        };
        let size = prop_u64(block, "Size");
        let mut name = prop_str(block, "IdLabel").to_string();
        if name.is_empty() {
            name = format!("{} Volume", crate::format::format_size(size));
        }
        let mut device = prop_bytes(block, "PreferredDevice");
        if device.as_os_str().is_empty() {
            device = prop_bytes(block, "Device");
        }
        volumes.push(Volume {
            id: path.clone(),
            drive_id: drive_id.to_string(),
            name,
            device,
            size,
            fs_type: prop_str(block, "IdType").to_string(),
            mount_point: mount_points(filesystem).into_iter().next(),
            can_power_off,
        });
    }
    return volumes;
}

/// A loop device (disk image) that user `uid` set up.
fn is_own_loop(interfaces: &BTreeMap<String, Props>, uid: u32) -> bool {
    let Some(loop_props) = interfaces.get(LOOP) else {
        return false;
    };
    return loop_props.get("SetupByUID").and_then(Value::as_u32) == Some(uid);
}

/// Filesystem.MountPoints: an array of NUL-terminated byte strings.
fn mount_points(filesystem: &Props) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(items) = filesystem.get("MountPoints").and_then(Value::as_array) else {
        return out;
    };
    for item in items {
        if let Some(bytes) = item.as_bytes() {
            let path = path_from_bytes(bytes);
            if !path.as_os_str().is_empty() {
                out.push(path);
            }
        }
    }
    return out;
}

/// Turns the reply of GetManagedObjects, `a{oa{sa{sv}}}`, into maps.
/// Anything shaped differently is skipped.
pub fn parse_objects(value: &Value) -> Objects {
    let mut objects = Objects::new();
    for entry in value.as_array().unwrap_or(&[]) {
        let Value::DictEntry(path, interfaces) = entry else {
            continue;
        };
        let Some(path) = path.as_str() else {
            continue;
        };
        let mut by_interface = BTreeMap::new();
        for entry in interfaces.as_array().unwrap_or(&[]) {
            let Value::DictEntry(name, props) = entry else {
                continue;
            };
            let Some(name) = name.as_str() else {
                continue;
            };
            let mut by_prop = Props::new();
            for entry in props.as_array().unwrap_or(&[]) {
                let Value::DictEntry(key, value) = entry else {
                    continue;
                };
                if let Some(key) = key.as_str() {
                    by_prop.insert(key.to_string(), value.unwrap_variant().clone());
                }
            }
            by_interface.insert(name.to_string(), by_prop);
        }
        objects.insert(path.to_string(), by_interface);
    }
    return objects;
}

fn managed_objects(timeout: Duration) -> io::Result<Objects> {
    let reply = call(
        ROOT,
        OBJECT_MANAGER,
        "GetManagedObjects",
        Vec::new(),
        timeout,
    )
    .map_err(io::Error::other)?;
    return match reply.body.first() {
        Some(value) => Ok(parse_objects(value)),
        None => Ok(Objects::new()),
    };
}

/// Calls a UDisks method on a fresh system bus connection and waits for
/// the reply. polkit may ask the user for a password meanwhile.
fn call(
    path: &str,
    interface: &str,
    member: &str,
    body: Vec<Value>,
    timeout: Duration,
) -> Result<Message, String> {
    let (mut connection, mut reader) = connect(timeout).map_err(|err| {
        return format!("can't reach the disk service (is udisks2 installed and running?): {err}");
    })?;
    let mut message = Message::method_call(SERVICE, path, interface, member, body);
    message.flags |= ALLOW_INTERACTIVE_AUTHORIZATION;
    return connection
        .call(&mut reader, &message)
        .map_err(|err| friendly_error(&err.to_string()));
}

fn connect(timeout: Duration) -> io::Result<(Connection, UnixStream)> {
    let (mut connection, mut reader) = Connection::open(&conn::system_address())?;
    // A service that never answers mustn't hang this thread forever.
    reader.set_read_timeout(Some(timeout))?;
    connection.hello(&mut reader)?;
    return Ok((connection, reader));
}

/// UDisks errors come as "org.freedesktop.UDisks2.Error.Name: text"; the
/// text is usually readable, the name only sometimes adds anything.
fn friendly_error(text: &str) -> String {
    if text.contains("NotAuthorized") {
        return "not allowed to do that (polkit said no)".to_string();
    }
    if text.contains("DeviceBusy") || text.contains("target is busy") {
        return "a program is still using files on it; close them and try again".to_string();
    }
    // A read timeout shows up as EAGAIN.
    if text.contains("temporarily unavailable") || text.contains("timed out") {
        return "the disk service didn't answer in time".to_string();
    }
    return match text.split_once(": ") {
        Some((_, rest)) if !rest.is_empty() => rest.to_string(),
        _ => text.to_string(),
    };
}

/// An empty `a{sv}`: UDisks methods all take an options dict.
fn no_options() -> Value {
    return Value::Array(
        Type::DictEntry(Box::new(Type::Str), Box::new(Type::Variant)),
        Vec::new(),
    );
}

fn prop_bool(props: &Props, key: &str) -> bool {
    return props.get(key).and_then(Value::as_bool).unwrap_or(false);
}

fn prop_u64(props: &Props, key: &str) -> u64 {
    return props.get(key).and_then(Value::as_u64).unwrap_or(0);
}

fn prop_str<'a>(props: &'a Props, key: &str) -> &'a str {
    return props.get(key).and_then(Value::as_str).unwrap_or("");
}

/// A NUL-terminated byte string property, as a path.
fn prop_bytes(props: &Props, key: &str) -> PathBuf {
    return match props.get(key).and_then(Value::as_bytes) {
        Some(bytes) => path_from_bytes(bytes),
        None => PathBuf::new(),
    };
}

/// Byte strings from UDisks end in one NUL (sometimes more); drop them.
fn path_from_bytes(mut bytes: Vec<u8>) -> PathBuf {
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    return PathBuf::from(OsString::from_vec(bytes));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: Value, value: Value) -> Value {
        return Value::DictEntry(Box::new(key), Box::new(value));
    }

    /// Builds a GetManagedObjects-shaped value from
    /// (path, [(interface, a{sv})]).
    fn objects_value(objects: Vec<(&str, Vec<(&str, Value)>)>) -> Value {
        let props_type = Type::Array(Box::new(Type::DictEntry(
            Box::new(Type::Str),
            Box::new(Type::Variant),
        )));
        let interfaces_type = Type::Array(Box::new(Type::DictEntry(
            Box::new(Type::Str),
            Box::new(props_type.clone()),
        )));
        let mut entries = Vec::new();
        for (path, interfaces) in objects {
            let mut inner = Vec::new();
            for (name, props) in interfaces {
                inner.push(entry(Value::Str(name.to_string()), props));
            }
            entries.push(entry(
                Value::ObjectPath(path.to_string()),
                Value::Array(
                    Type::DictEntry(Box::new(Type::Str), Box::new(props_type.clone())),
                    inner,
                ),
            ));
        }
        return Value::Array(
            Type::DictEntry(Box::new(Type::ObjectPath), Box::new(interfaces_type)),
            entries,
        );
    }

    fn mount_points_value(points: &[&str]) -> Value {
        let mut items = Vec::new();
        for point in points {
            let mut bytes = point.as_bytes().to_vec();
            bytes.push(0);
            items.push(Value::bytes(&bytes));
        }
        return Value::Array(Type::Array(Box::new(Type::Byte)), items);
    }

    fn block(drive: &str, label: &str, system: bool, ignore: bool) -> Value {
        return Value::dict(vec![
            ("Drive", Value::ObjectPath(drive.to_string())),
            ("IdLabel", Value::Str(label.to_string())),
            ("IdType", Value::Str("vfat".to_string())),
            ("Size", Value::U64(16 * 1024 * 1024 * 1024)),
            ("HintSystem", Value::Bool(system)),
            ("HintIgnore", Value::Bool(ignore)),
            ("PreferredDevice", Value::bytes(b"/dev/sdb1\0")),
        ]);
    }

    fn sample() -> Objects {
        let usb = "/org/freedesktop/UDisks2/drives/SanDisk_123";
        let value = objects_value(vec![
            (
                "/org/freedesktop/UDisks2/block_devices/sdb1",
                vec![
                    (BLOCK, block(usb, "STICK", false, false)),
                    (
                        FILESYSTEM,
                        Value::dict(vec![("MountPoints", mount_points_value(&[]))]),
                    ),
                ],
            ),
            (
                "/org/freedesktop/UDisks2/block_devices/sdb2",
                vec![
                    (BLOCK, block(usb, "", false, false)),
                    (
                        FILESYSTEM,
                        Value::dict(vec![(
                            "MountPoints",
                            mount_points_value(&["/media/me/DATA"]),
                        )]),
                    ),
                ],
            ),
            (
                // An internal disk: UDisks says it's a system one.
                "/org/freedesktop/UDisks2/block_devices/nvme0n1p5",
                vec![
                    (
                        BLOCK,
                        block(
                            "/org/freedesktop/UDisks2/drives/Samsung",
                            "Windows",
                            true,
                            false,
                        ),
                    ),
                    (
                        FILESYSTEM,
                        Value::dict(vec![("MountPoints", mount_points_value(&[]))]),
                    ),
                ],
            ),
            (
                // The whole stick: a block without a filesystem.
                "/org/freedesktop/UDisks2/block_devices/sdb",
                vec![(BLOCK, block(usb, "", false, false))],
            ),
            (
                usb,
                vec![(
                    DRIVE,
                    Value::dict(vec![
                        ("CanPowerOff", Value::Bool(true)),
                        ("Ejectable", Value::Bool(false)),
                    ]),
                )],
            ),
        ]);
        return parse_objects(&value);
    }

    #[test]
    fn picks_removable_filesystems() {
        let volumes = volumes_from_objects(&sample(), 1000);
        assert_eq!(volumes.len(), 2);
        let stick = &volumes[0];
        assert_eq!(stick.name, "STICK");
        assert_eq!(stick.device, PathBuf::from("/dev/sdb1"));
        assert_eq!(stick.fs_type, "vfat");
        assert_eq!(stick.mount_point, None);
        assert!(stick.can_power_off);
        assert_eq!(
            stick.drive_id,
            "/org/freedesktop/UDisks2/drives/SanDisk_123"
        );
        // No label: named by size.
        assert_eq!(volumes[1].name, "16 GB Volume");
        assert_eq!(
            volumes[1].mount_point,
            Some(PathBuf::from("/media/me/DATA"))
        );
    }

    #[test]
    fn own_disk_images_show_up() {
        let image = |uid: u32| {
            objects_value(vec![(
                "/org/freedesktop/UDisks2/block_devices/loop0",
                vec![
                    (BLOCK, block("/", "IMAGE", true, false)),
                    (
                        FILESYSTEM,
                        Value::dict(vec![("MountPoints", mount_points_value(&[]))]),
                    ),
                    (LOOP, Value::dict(vec![("SetupByUID", Value::U32(uid))])),
                ],
            )])
        };
        let mine = volumes_from_objects(&parse_objects(&image(1000)), 1000);
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].name, "IMAGE");
        assert_eq!(mine[0].drive_id, "");
        assert!(!mine[0].can_power_off);
        // Someone else's (or root's) image stays hidden.
        assert!(volumes_from_objects(&parse_objects(&image(0)), 1000).is_empty());
    }

    #[test]
    fn eject_unmounts_everything_mounted_on_the_drive() {
        let objects = sample();
        let volumes = volumes_from_objects(&objects, 1000);
        // Ejecting via the unmounted partition still unmounts its mounted
        // sibling, which is what makes pulling the stick safe.
        assert_eq!(
            blocks_to_unmount(&objects, &volumes[0]),
            vec!["/org/freedesktop/UDisks2/block_devices/sdb2".to_string()]
        );
    }

    #[test]
    fn odd_shapes_are_skipped() {
        assert!(parse_objects(&Value::Str("nope".to_string())).is_empty());
        assert!(parse_objects(&Value::bytes(b"abc")).is_empty());
        let objects = parse_objects(&objects_value(vec![(
            "/x",
            vec![(BLOCK, Value::Str("not a dict".to_string()))],
        )]));
        assert!(volumes_from_objects(&objects, 1000).is_empty());
    }

    #[test]
    fn errors_read_well() {
        assert_eq!(
            friendly_error(
                "org.freedesktop.UDisks2.Error.DeviceBusy: Error unmounting: target is busy"
            ),
            "a program is still using files on it; close them and try again"
        );
        assert_eq!(
            friendly_error(
                "org.freedesktop.UDisks2.Error.Failed: Error mounting /dev/sdb1: wrong fs type"
            ),
            "Error mounting /dev/sdb1: wrong fs type"
        );
        assert!(
            friendly_error("org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: x")
                .contains("polkit")
        );
        assert_eq!(
            path_from_bytes(b"/dev/sdb1\0\0".to_vec()),
            PathBuf::from("/dev/sdb1")
        );
    }
}

/// Talks to the real UDisks (read-only). Run by hand:
/// `cargo test -p itamiiru-core live_udisks -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn live_udisks() {
        let objects = managed_objects(LIST_TIMEOUT).expect("UDisks answers");
        let mut blocks = 0;
        for (path, interfaces) in &objects {
            if let Some(block) = interfaces.get(BLOCK) {
                blocks += 1;
                println!(
                    "{path}: system={} ignore={} filesystem={} type={}",
                    prop_bool(block, "HintSystem"),
                    prop_bool(block, "HintIgnore"),
                    interfaces.contains_key(FILESYSTEM),
                    prop_str(block, "IdType"),
                );
            }
        }
        assert!(blocks > 0, "UDisks knows no block devices");
        println!("removable volumes: {:#?}", removable_volumes());
    }

    /// Mounts and ejects a real volume. Set it up with a throwaway image:
    /// `truncate -s 16M t.img && mkfs.ext4 -F t.img &&
    /// udisksctl loop-setup -f t.img`, then
    /// `ITAMI_TEST_DEVICE=/dev/loopN cargo test -p itamiiru-core
    /// live_mount_and_eject -- --ignored --nocapture`. Ejecting detaches the
    /// image; `udisksctl loop-delete -b /dev/loopN` if the test stops early.
    #[test]
    #[ignore]
    fn live_mount_and_eject() {
        let Some(device) = std::env::var_os("ITAMI_TEST_DEVICE") else {
            println!("ITAMI_TEST_DEVICE isn't set; skipping");
            return;
        };
        let device = PathBuf::from(device);
        let find = || {
            for volume in removable_volumes() {
                if volume.device == device {
                    return Some(volume);
                }
            }
            return None;
        };
        let volume = find().expect("the test device is listed");
        println!("found: {volume:#?}");
        assert_eq!(volume.mount_point, None, "start with it unmounted");

        let point = mount_volume(&volume).expect("mounts");
        println!("mounted at {}", point.display());
        assert!(
            point.join("lost+found").exists(),
            "the filesystem is readable"
        );
        let mounted = find().expect("still listed");
        assert_eq!(mounted.mount_point.as_deref(), Some(point.as_path()));

        // Unmount: still plugged in and listed, just not mounted.
        unmount_volume(&mounted).expect("unmounts");
        let unmounted = find().expect("still listed after unmounting");
        assert_eq!(unmounted.mount_point, None);
        println!("unmounted; still listed");
        let point = mount_volume(&unmounted).expect("mounts again");
        let mounted = find().expect("listed");
        assert_eq!(mounted.mount_point.as_deref(), Some(point.as_path()));

        eject_volume(&mounted).expect("ejects");
        // Unmounted, and a disk image of ours is detached too.
        let after = find();
        assert!(
            after.as_ref().is_none_or(|v| v.mount_point.is_none()),
            "{after:?}"
        );
        assert!(!point.exists() || std::fs::read_dir(&point).map(|d| d.count()).unwrap_or(0) == 0);
        println!("ejected; mount folder gone or empty");
    }
}
