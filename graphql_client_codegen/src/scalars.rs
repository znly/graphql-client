use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use std::cell::Cell;

#[derive(Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
pub struct Scalar<'schema> {
    pub name: &'schema str,
    pub description: Option<&'schema str>,
    pub is_required: Cell<bool>,
}

impl<'schema> Scalar<'schema> {
    // TODO: do something smarter here
    pub fn to_rust(&self) -> TokenStream {
        let name = self.name;
        #[cfg(feature = "normalize_query_types")]
        let name = {
            use heck::CamelCase;

            name.to_camel_case()
        };
        let ident = Ident::new(&name, Span::call_site());
        let description = &self.description.map(|d| quote!(#[doc = #d]));

        quote!(#description type #ident = super::#ident;)
    }

    pub fn to_go(&self) -> TokenStream {
        let from = Ident::new(&self.name, Span::call_site());

        match self.name {
            "Duration" => quote! {
                type #from time.Duration;

                func (d *#from) UnmarshalJSON(payload []byte) error {
                    var val int;
                    var err error;
                    val, err = strconv.Atoi(string(payload));
                    if err != nil {
                        return err;
                    };
                    var ms = time.Millisecond * time.Duration(val);
                    *d = #from(ms.Nanoseconds());
                    return nil;
                }
            },
            "MidnightOffset" => quote! {
                type #from time.Duration;

                func (d *#from) UnmarshalJSON(payload []byte) error {
                    var val int;
                    var err error;
                    val, err = strconv.Atoi(string(payload));
                    if err != nil {
                        return err;
                    };
                    var s = time.Second * time.Duration(val);
                    *d = #from(s.Nanoseconds());
                    return nil;
                }
            },
            "Timezone" => quote! {
                type #from String
            },
            "DateTimeUtc" => quote! {
                type #from time.Time;

                func (d *#from) UnmarshalJSON(payload []byte) error {
                    var val time.Time;
                    var err error;
                    var s string;
                    s, _ = strconv.Unquote(string(payload));
                    val, err = time.Parse(time.RFC3339, s);
                    if err != nil {
                        return err;
                    };
                    *d = #from(val);
                    return nil;
                };

                func (d *#from) MarshalJSON() ([]byte, error) {
                    return json.Marshal(time.Time(*d).Format(time.RFC3339));
                }

            },
            _ => quote!(type #from = #from),
        }
    }
}
