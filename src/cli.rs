use clap::ValueEnum;

#[derive(ValueEnum, Clone, Debug)]
pub enum VerifyMode {
    Auto,
    On,
    Off,
}
