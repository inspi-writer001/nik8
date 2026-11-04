use std::{
    fs::{File, create_dir_all},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::state::Validator;

/// HashMap<String, Vec<usize>> is a hashmap of each validators assigned to be leaders in this epoch.
///
/// an Epoch can have 0 - 431999 Slots ..i.e. `432000` slots in total.
// each validator has an array to show how many slots `offset` they'll be Leaders for, for example
/// at epoch 1605,
/// validator `3Y4wkURAahySj7p8GuMjXSYosqTvP7UqPj7yjhRxgVoJ` is leader in [804, 805, 806, 807, 2932, 2933, 2934, 2935, 3972, 3973, 3974, 3975, 4200, 4201, 4202, 4203, 5556, 5557, 5558, 5559,...] `offsets` in that epoch
pub async fn write_leaders_to_file(
    epoch: u64,
    leaders: Arc<std::collections::HashMap<String, Vec<usize>>>,
) -> std::io::Result<PathBuf> {
    let duration_since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;

    let elapsed_nano = duration_since.as_nanos();
    let file_name = format!("{epoch}-{elapsed_nano}");
    let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("leader_schedule");

    create_dir_all(&output_dir)?;
    let output_path = output_dir.join(file_name);
    let file = File::create(&output_path)?;

    let mut writer = BufWriter::new(file);
    for (validator, slots) in leaders.iter() {
        writeln!(writer, "{validator}:{slots:?}")?;
    }

    writer.flush()?;
    Ok(output_path)
}

pub async fn write_ports_to_file(
    epoch: u64,
    leaders: Arc<Vec<Validator>>,
) -> std::io::Result<PathBuf> {
    let duration_since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;

    let elapsed_nano = duration_since.as_nanos();
    let file_name = format!("{epoch}-{elapsed_nano}");
    let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cluster_nodes");

    create_dir_all(&output_dir)?;
    let output_path = output_dir.join(file_name);
    let file = File::create(&output_path)?;

    let mut writer = BufWriter::new(file);

    for validator in leaders.iter() {
        let key = validator.address.clone();
        let value = validator;

        // write!(writer, "{key}");
        writeln!(writer, "{key}:{value:?}")?;
    }

    writer.flush()?;
    Ok(output_path)
}
