extern crate proc_macro;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashMap;

#[derive(Debug, PartialEq)]
enum Error {
    IoError(String),
    ParseError(String),
    MissingQueryDefinition,
}

impl From<graphql_parser::schema::ParseError> for Error {
    fn from(e: graphql_parser::schema::ParseError) -> Error {
        Error::ParseError(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::IoError(e.to_string())
    }
}

#[derive(Debug)]
struct QueryDslParams {
    schema_filename: String,
}

impl QueryDslParams {
    fn new(schema_filename: String) -> Self {
        QueryDslParams { schema_filename }
    }
}

impl syn::parse::Parse for QueryDslParams {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input
            .parse::<syn::LitStr>()
            .map(|lit_str| QueryDslParams::new(lit_str.value()))
    }
}

#[proc_macro]
pub fn query_dsl(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as QueryDslParams);

    let rv = query_dsl_from_schema(input).unwrap().into();
    println!("{}", rv);
    rv
}

fn query_dsl_from_schema(input: QueryDslParams) -> Result<TokenStream, Error> {
    use graphql_parser::schema::Definition;

    let schema = std::fs::read_to_string(&input.schema_filename)?;
    let schema_data = data_from_schema(graphql_parser::schema::parse_schema(&schema)?);

    let objects: Vec<_> = schema_data
        .types
        .iter()
        .map(|(_, v)| dsl_for_object(v))
        .collect();

    Ok(quote! {
        #(
            #objects
        )*
    })
}

fn dsl_for_object(object: &graphql_parser::schema::ObjectType) -> TokenStream {
    let struct_name = format_ident!("{}", object.name);

    let function_tokens: Vec<_> = object
        .fields
        .iter()
        .map(|f| select_function_for_field(f, &struct_name))
        .collect();

    quote! {
        pub struct #struct_name;

        impl #struct_name {
            #(
                #function_tokens
            )*
        }
    }
}

fn select_function_for_field(
    field: &graphql_parser::schema::Field,
    type_lock: &proc_macro2::Ident,
) -> TokenStream {
    use graphql_parser::schema::Type;
    use inflector::Inflector;

    let query_field_name = syn::LitStr::new(&field.name, Span::call_site());
    let rust_field_name = format_ident!("{}", field.name.to_snake_case());

    let (field_type, scalar_call) = field_type_and_scalar_call(&field.field_type);

    if let Some(scalar_call) = scalar_call {
        quote! {
            pub fn #rust_field_name() -> ::cynic::selection_set::SelectionSet<'static, #field_type, #type_lock> {
                use ::cynic::selection_set::{string, integer, float, boolean};

                ::cynic::selection_set::field(#query_field_name, #scalar_call)
            }
        }
    } else {
        quote! {
            pub fn #rust_field_name<'a, T>(fields: ::cynic::selection_set::SelectionSet<'a, T, #field_type>)
                -> ::cynic::selection_set::SelectionSet<T, #type_lock>
                where T: 'a {
                    ::cynic::selection_set::field(#query_field_name, fields)
                }
        }
    }
}

enum FieldType {
    Enum(syn::Type, String),
    Object(syn::Type),
    List(syn::Type),
}

fn field_type_and_scalar_call(
    gql_type: &graphql_parser::schema::Type,
) -> (TokenStream, Option<TokenStream>) {
    use graphql_parser::schema::Type;

    // TODO: Need to update this to support custom scalars.
    match gql_type {
        Type::NonNullType(inner_type) => {
            let (inner_type, scalar_call) = field_type_and_scalar_call(inner_type);
            (
                quote! { Option<#inner_type> },
                scalar_call.map(|expr| quote! { ::cynic::selection_set::option(#expr) }),
            )
        }
        Type::ListType(inner_type) => {
            let (inner_type, scalar_call) = field_type_and_scalar_call(inner_type);
            (
                quote! { Vec<#inner_type> },
                scalar_call.map(|expr| quote! { ::cynic::selection_set::vec(#expr) }),
            )
        }
        Type::NamedType(name) => {
            let (field_type, scalar_func) = if name == "String" {
                ("String".to_string(), Some("string"))
            } else if name == "Int" {
                ("i64".to_string(), Some("integer"))
            } else if name == "Float" {
                ("f64".to_string(), Some("float"))
            } else if name == "Boolean" {
                ("bool".to_string(), Some("boolean"))
            } else if name == "String" {
                // TODO: Could do something more sensible for IDs here...
                ("String".to_string(), Some("string"))
            } else {
                (name.to_string(), None)
            };

            let field_type = format_ident!("{}", field_type);
            let scalar_func = scalar_func.map(|f| format_ident!("{}", f));

            (quote! { #field_type }, scalar_func.map(|f| quote! { #f() }))
        }
    }
}

#[derive(Debug)]
struct SchemaData {
    types: HashMap<String, graphql_parser::schema::ObjectType>,
}

fn data_from_schema(document: graphql_parser::schema::Document) -> SchemaData {
    use graphql_parser::schema::{Definition, TypeDefinition};

    let mut types = HashMap::new();

    for definition in document.definitions {
        match definition {
            Definition::TypeDefinition(TypeDefinition::Object(object)) => {
                types.insert(object.name.clone(), object.clone());
            }
            _ => {}
        }
    }

    SchemaData { types }
}