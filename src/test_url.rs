fn main() {
    let s = "https://img.vietqr.io/image/MB Bank-123-print.png?amount=100&addInfo=test";
    match url::Url::parse(s) {
        Ok(u) => println!("OK"),
        Err(e) => println!("Err: {}", e),
    }
}
