use serde;
use std;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ApiVersion {
    pub major: i64,
    pub minor: i64,
    pub patch: i64,
    pub api_level: i64,
    pub api_compatible: i64,
    pub api_prerelease: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ApiType {
    Nil,
    Boolean,
    Integer,
    Float,
    String,
    Array,
    ArrayOf(Box<ApiType>),
    ArrayOfLength(Box<ApiType>, i64),
    Dictionary,
    Buffer,
    Window,
    Tabpage,
    // NB: this is not listed in the neovim rpc-types docs, but it does occur in the
    // auto-generated API description.
    Object,
}

impl ApiType {
    /*
    pub fn rust_borrowed_name(&self) -> String {
        use itertools::Itertools;
        use self::ApiType::*;

        match *self {
            Nil => "()".into(),
            Boolean => "bool".into(),
            Integer => "i64".into(),
            Float => "f64".into(),
            String => "&str".into(),
            Array => "&[Value]".into(),
            ArrayOf(ref ty) => format!("&[{}]", ty.rust_borrowed_name()),
            ArrayOfLength(ref ty, num) => {
                let ty_str = ty.rust_borrowed_name();
                format!("({})", std::iter::repeat(ty_str).take(num as usize).format(", "))
            },
            Dictionary => "&[(Value, Value)]".into(),
            Buffer => "Buffer".into(),
            Window => "Window".into(),
            Tabpage => "Tabpage".into(),
            Object => "&Value".into(),
        }
    }
    */

    pub fn rust_owned_name(&self) -> String {
        use itertools::Itertools;
        use ApiType::*;

        match *self {
            Nil => "()".into(),
            Boolean => "bool".into(),
            Integer => "i64".into(),
            Float => "f64".into(),
            String => "String".into(),
            Array => "Vec<Value>".into(),
            ArrayOf(ref ty) => format!("Vec<{}>", ty.rust_owned_name()),
            ArrayOfLength(ref ty, num) => {
                let ty_str = ty.rust_owned_name();
                format!("({})", std::iter::repeat(ty_str).take(num as usize).format(", "))
            },
            Dictionary => "Map".into(),
            Object => "Value".into(),
            // TODO: these shouldn't be hard-coded, but derived from nvim's API specification
            Buffer => "Buffer<'client>".into(),
            Window => "Window<'client>".into(),
            Tabpage => "Tabpage<'client>".into(),
        }
    }
}

struct ApiTypeVisitor;

impl ApiTypeVisitor {
    fn parse_primitive_type<E: serde::de::Error>(&self, value: &str) -> Result<ApiType, E> {
        match value {
            // NB: this doesn't seem to be mentioned in the neovim rpc-types docs, but 'void'
            // occurs a lot in the auto-generated API description
            "void" => Ok(ApiType::Nil),
            "Nil" => Ok(ApiType::Nil),
            "Boolean" => Ok(ApiType::Boolean),
            "Integer" => Ok(ApiType::Integer),
            "Float" => Ok(ApiType::Float),
            "String" => Ok(ApiType::String),
            "Array" => Ok(ApiType::Array),
            "Dictionary" => Ok(ApiType::Dictionary),
            "Buffer" => Ok(ApiType::Buffer),
            "Window" => Ok(ApiType::Window),
            "Tabpage" => Ok(ApiType::Tabpage),
            "Object" => Ok(ApiType::Object),
            _ => Err(E::unknown_variant(value, &["a valid neovim API type"]))
        }
    }

    fn parse_type<E: serde::de::Error>(&self, value: &str) -> Result<ApiType, E> {
        if value.starts_with("ArrayOf") {
            let e = || E::unknown_variant(value, &["a valid neovim API type"]);
            // We expect one of two formats: ArrayOf(<type>) or ArrayOf(<type>, <length>).
            let open = value.find('(').ok_or(e())?;
            let close = value.rfind(')').ok_or(e())?;
            let args = &value[(open + 1)..close];

            // NB: in principle, we should support recursive types
            // (e.g.  ArrayOf(ArrayOf(Integer, 2), 3)). However, that's rather more complicated and
            // anyway neovim doesn't have any of those types in its API.
            match args.find(',') {
                Some(comma) => {
                    let (ty, num) = args.split_at(comma);
                    let num = &num[1..]; // trim the comma
                    let ty = self.parse_primitive_type(ty.trim())?;
                    let num = num.trim().parse::<i64>().map_err(|_| e())?;
                    Ok(ApiType::ArrayOfLength(Box::new(ty), num))
                },
                None => {
                    let ty = self.parse_primitive_type(args.trim())?;
                    Ok(ApiType::ArrayOf(Box::new(ty)))
                }
            }
        } else {
            self.parse_primitive_type(value)
        }
    }
}

impl<'de> serde::de::Visitor<'de> for ApiTypeVisitor {
    type Value = ApiType;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a byte string representing one of neovim's API types")
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, value: &'de str) -> Result<ApiType, E> {
        self.parse_type(value)
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<ApiType, E> {
        self.parse_type(value)
    }
}

impl<'de> serde::Deserialize<'de> for ApiType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        deserializer.deserialize_any(ApiTypeVisitor)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
pub struct ApiFunctionParam<'a> {
    pub ty: ApiType,
    pub name: &'a str,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
pub struct ApiFunction<'a> {
    pub name: &'a str,
    #[serde(borrow)]
    pub parameters: Vec<ApiFunctionParam<'a>>,
    pub return_type: ApiType,
    pub method: bool,
    pub since: i64,
    pub deprecated_since: Option<i64>,
}

impl<'a> ApiFunction<'a> {
    fn rust_fn_name(&self, prefix: &str) -> String {
        assert!(self.name.starts_with(prefix));
        self.name[prefix.len()..].to_owned()
    }

    pub fn macro_call(&self, fn_prefix: &str) -> String {
        let num_to_skip = if self.method { 1 } else { 0 };
        let macro_name = if self.method { "nvim_api_method" } else { "nvim_api_function" };
        let param_strings = self.parameters.iter()
            .skip(num_to_skip)
            // Note that we're using owned names, because rmpv prefers owned names for converting
            // into Values.
            .map(|p| format!("{}: {}", p.name, p.ty.rust_owned_name()))
            .collect::<Vec<_>>();

        format!(r#"{macro_name}!({fn_name}, "{nvim_fn_name}", {params}; {ret_ty});"#,
            macro_name = macro_name,
            fn_name = self.rust_fn_name(fn_prefix),
            nvim_fn_name = self.name,
            params = param_strings.join(", "),
            ret_ty = self.return_type.rust_owned_name()
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
pub struct ApiUiEvent<'a> {
    pub name: &'a str,
    #[serde(borrow)]
    pub parameters: Vec<ApiFunctionParam<'a>>,
    pub since: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
pub struct ApiTypeDecl<'a> {
    pub prefix: &'a str,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Api<'a> {
    version: ApiVersion,
    #[serde(borrow)]
    pub functions: Vec<ApiFunction<'a>>,
    #[serde(borrow)]
    ui_events: Vec<ApiUiEvent<'a>>,
    #[serde(borrow)]
    pub types: HashMap<&'a str, ApiTypeDecl<'a>>,
}


