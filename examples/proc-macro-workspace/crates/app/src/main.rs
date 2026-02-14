use hello_macro::forty_two;

fn main() {
    println!("{} {}", genmsg::generated_message(), forty_two!());
}

