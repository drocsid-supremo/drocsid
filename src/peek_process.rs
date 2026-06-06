pub fn peek_process_ctx(args: &Vec<String>) -> (&str, &str) {
    let mut mode = "";
    let mut username = "";

    for arg in args {
        if arg.starts_with("mode=") {
            mode = &arg[5..];
        }

        if arg.starts_with("username=") {
            username = &arg[9..];
        }
    }

    (mode, username)
}