use proc_macro::TokenStream;

#[proc_macro]
pub fn forty_two(_input: TokenStream) -> TokenStream {
    "42".parse().expect("static token stream must parse")
}

