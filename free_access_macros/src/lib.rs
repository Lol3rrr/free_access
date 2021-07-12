use proc_macro::TokenStream;
use syn::{parse_macro_input, AttributeArgs};

mod wrapper;

#[proc_macro_attribute]
pub fn freeaccess(attr: TokenStream, input: TokenStream) -> TokenStream {
    let input_impl: syn::ItemFn = parse_macro_input!(input);
    let attributes = parse_macro_input!(attr as AttributeArgs);

    wrapper::wrapper(attributes, input_impl).into()
}
