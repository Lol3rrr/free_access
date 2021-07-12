use proc_macro2::TokenStream;
use quote::quote;

pub fn wrapper(_attributes: syn::AttributeArgs, input: syn::ItemFn) -> TokenStream {
    quote! {
        #input
    }
}
