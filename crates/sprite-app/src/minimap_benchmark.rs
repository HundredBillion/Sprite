use std::{
    fs::OpenOptions,
    io::Write,
    sync::{OnceLock, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn record(mut value: serde_json::Value) {
    static LOG: OnceLock<Option<mpsc::Sender<serde_json::Value>>> = OnceLock::new();
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    value["unix_ns"] = (now.as_nanos().min(u64::MAX as u128) as u64).into();
    let Some(sender) = LOG.get_or_init(|| {
        let path = std::env::var("SPRITE_MINIMAP_BENCHMARK_LOG").ok()?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let (sender, receiver) = mpsc::channel::<serde_json::Value>();
        // Keep diagnostics from delaying the input and paint being measured.
        std::thread::Builder::new()
            .name("sprite-minimap-benchmark".into())
            .spawn(move || {
                for value in receiver {
                    let Ok(mut line) = serde_json::to_vec(&value) else {
                        break;
                    };
                    line.push(b'\n');
                    if let Err(error) = file.write_all(&line) {
                        eprintln!("minimap benchmark log: {error}");
                        break;
                    }
                }
            })
            .ok()?;
        Some(sender)
    }) else {
        return;
    };
    sender.send(value).ok();
}
