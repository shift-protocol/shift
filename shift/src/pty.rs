use std::os::unix::io::RawFd;
use termios::{self, Termios};

pub fn enable_raw_mode(fd: RawFd) -> Termios {
    let mut termios = Termios::from_fd(fd).unwrap();
    let old_termios = termios;
    termios.c_iflag &= !(termios::PARMRK
        | termios::ISTRIP
        | termios::INLCR
        | termios::IGNCR
        | termios::ICRNL
        | termios::IXON);
    termios.c_lflag &= !(termios::ECHO | termios::ECHONL | termios::ICANON | termios::IEXTEN);
    termios.c_cflag &= !(termios::CSIZE | termios::PARENB);
    termios.c_cflag |= termios::CS8;
    termios.c_cc[termios::VMIN] = 1;
    termios::tcsetattr(0, termios::TCSANOW, &termios).unwrap();
    return old_termios;
}

pub fn restore_mode(fd: RawFd, old_termios: Termios) {
    termios::tcsetattr(fd, termios::TCSANOW, &old_termios).unwrap();
}
