pub fn peek_process_ctx(args: &[String]) -> (&str, &str) {
    let mut mode = "";
    let mut username = "";

    for arg in args {
        if let Some(value) = arg.strip_prefix("mode=") {
            mode = value;
        }

        if let Some(value) = arg.strip_prefix("username=") {
            username = value;
        }
    }

    (mode, username)
}
