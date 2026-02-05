fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (left_tx, left_rx) = std::sync::mpsc::channel();
    let _output = meshcq_modem::device::start_default_output(left_rx)?;
    let _input = meshcq_modem::device::start_default_input(left_tx)?;
    std::thread::park();
    Ok(())
}
