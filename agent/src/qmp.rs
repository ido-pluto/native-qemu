use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Minimal synchronous QMP (QEMU Machine Protocol) client. Used for
/// graceful shutdown requests and USB hotplug device_add/device_del — the
/// main "did the guest shut down" signal is still just the qemu child
/// process exiting, which is the normal behavior on guest ACPI shutdown.
pub struct Qmp {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Qmp {
    pub fn connect(socket_path: &str) -> std::io::Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut qmp = Qmp { stream, reader };
        qmp.read_value()?; // greeting
        qmp.execute("qmp_capabilities", None)?;
        Ok(qmp)
    }

    fn read_value(&mut self) -> std::io::Result<Value> {
        let mut de = serde_json::Deserializer::from_reader(&mut self.reader);
        Value::deserialize(&mut de).map_err(std::io::Error::other)
    }

    /// Sends a command and reads values until a return/error reply arrives,
    /// discarding any asynchronous events seen in between.
    pub fn execute(&mut self, command: &str, arguments: Option<Value>) -> std::io::Result<Value> {
        let mut msg = json!({ "execute": command });
        if let Some(args) = arguments {
            msg["arguments"] = args;
        }
        let mut payload = serde_json::to_vec(&msg).map_err(std::io::Error::other)?;
        payload.push(b'\n');
        self.stream.write_all(&payload)?;

        loop {
            let value = self.read_value()?;
            if value.get("event").is_some() {
                continue;
            }
            if let Some(err) = value.get("error") {
                return Err(std::io::Error::other(format!("QMP error: {err}")));
            }
            return Ok(value);
        }
    }

    pub fn system_powerdown(&mut self) -> std::io::Result<()> {
        self.execute("system_powerdown", None).map(|_| ())
    }

    pub fn device_add(
        &mut self,
        driver: &str,
        bus_num: u16,
        device_num: u16,
        bus: &str,
        id: &str,
    ) -> std::io::Result<()> {
        self.execute(
            "device_add",
            Some(json!({
                "driver": driver,
                "hostbus": bus_num,
                "hostaddr": device_num,
                "bus": bus,
                "id": id,
            })),
        )
        .map(|_| ())
    }
}
