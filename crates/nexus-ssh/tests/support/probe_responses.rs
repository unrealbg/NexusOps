//! Fixed Linux responses shared by automated and manual SSH fixtures.
//! The input is matched as bytes and is never interpreted or executed.

pub fn response(command: &[u8]) -> Option<&'static str> {
    match command {
        b"uname -n" => Some("nexus-fixture\n"),
        b"cat /etc/os-release" => {
            Some("NAME=\"Fixture Linux\"\nVERSION_ID=\"1\"\nPRETTY_NAME=\"Fixture Linux 1\"\n")
        }
        b"uname -r" => Some("6.12.0-fixture\n"),
        b"uname -m" => Some("x86_64\n"),
        b"cat /proc/uptime" => Some("12345.50 20000.00\n"),
        b"cat /proc/loadavg" => Some("0.25 0.50 0.75 1/20 42\n"),
        b"cat /proc/meminfo" => {
            Some("MemTotal: 8192000 kB\nMemFree: 1024000 kB\nMemAvailable: 4096000 kB\n")
        }
        b"LC_ALL=C df -Pk /" => Some(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/test 1000000 250000 750000 25% /\n",
        ),
        _ => None,
    }
}
