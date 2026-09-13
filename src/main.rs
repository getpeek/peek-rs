fn main() {
    env_logger::init();
    peek_ui::run(peek_ui::Launch::from_args(std::env::args()));
}
