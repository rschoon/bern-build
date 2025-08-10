
use std::{collections::HashMap, ffi::OsStr, fmt::{self, Display as _}, fs, io::{BufReader, Read, Write}, path::Path, sync::Arc};

use anyhow::Context as _;
use minijinja::{value::{DynObject, Object}, Value};

#[derive(Debug)]
pub struct Environment {
    environment: minijinja::Environment<'static>,
    vars: HashMap<String, minijinja::Value>
}

impl Environment {
    pub fn new<P>(root: P) -> Self
    where 
        P: AsRef<Path>,
    {
        let mut environment = minijinja::Environment::empty();
        environment.set_loader(minijinja::path_loader(root));
        environment.set_undefined_behavior(minijinja::UndefinedBehavior::SemiStrict);
        register_filters(&mut environment);
        register_functions(&mut environment);
        register_tests(&mut environment);
        Self {
            environment,
            vars: HashMap::new(),
        }
    }

    pub fn set<V>(&mut self, name: String, value: V)
    where 
        V: Into<minijinja::Value>
    {
        self.vars.insert(name, value.into());
    }

    pub fn render_to(&self, src: &Path, w: impl Write) -> anyhow::Result<()> {
        let name = src.file_name().unwrap_or_else(|| OsStr::new("<input>")).to_string_lossy();
        let mut f = BufReader::new(fs::File::open(src)
            .with_context(|| format!("Failed to read file: {}", src.display()))?);

        let mut template = String::new();
        f.read_to_string(& mut template)?;

        let tpl = self.environment.template_from_named_str(&name, &template)?;

        tpl.render_to_write(&self.vars, w)?;

        Ok(())
    }
}

#[derive(Debug)]
struct DateTime(chrono::DateTime<chrono::Local>);

impl DateTime {
    pub fn now() -> Self {
        Self(chrono::Local::now())
    }

    pub fn format(&self, format: &str) -> String {
        self.0.format(format).to_string()
    }

    pub fn timestamp(&self) -> i64 {
        self.0.timestamp()
    }
}

impl Object for DateTime {
    fn call_method(
        self: &Arc<Self>,
        state: &minijinja::State<'_, '_>,
        method: &str,
        args: &[Value],
    ) -> Result<Value, minijinja::Error> {
        let this = self.clone();
        let method = if method == "format" {
            if args.is_empty() {
                Value::from_function(move || this.format("%+"))
            } else {
                Value::from_function(move |s: &str| this.format(s))
            }
        } else if method == "timestamp" {
            Value::from_function(move || this.timestamp())
        } else {
            return Err(minijinja::Error::from(minijinja::ErrorKind::UnknownMethod))
        };

        method.call(state, args)
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result
    where
        Self: Sized + 'static,
    {
        self.0.fmt(f)
    }
}

pub trait IntoValue {
    fn into_value(self) -> Result<Value, minijinja::Error>;
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Result<Value, minijinja::Error> {
        match self {
            Some(v) => v.into_value(),
            None => Ok(None::<()>.into())
        }
    }
}

impl<T: IntoValue> IntoValue for Result<T, anyhow::Error> {
    fn into_value(self) -> Result<Value, minijinja::Error> {
        match self {
            Ok(v) => v.into_value(),
            Err(e) => Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
        }
    }
}

impl<T: Object + 'static> IntoValue for Arc<T> {
    fn into_value(self) -> Result<Value, minijinja::Error> {
        Ok(DynObject::new(self).into())
    }
}

impl IntoValue for () {
    fn into_value(self) -> Result<Value, minijinja::Error> {
        Ok(().into())
    }
}

fn register_filters(env: &mut minijinja::Environment) {
    use minijinja::filters::*;

    env.add_filter("abs", abs);
    env.add_filter("attr", attr);
    env.add_filter("batch", batch);
    env.add_filter("bool", bool);
    env.add_filter("capitalize", capitalize);
    env.add_filter("default", default);
    env.add_filter("dictsort", dictsort);
    env.add_filter("first", first);
    env.add_filter("float", float);
    env.add_filter("groupby", groupby);
    env.add_filter("indent", indent);
    env.add_filter("int", int);
    env.add_filter("items", items);
    env.add_filter("join", join);
    env.add_filter("last", last);
    env.add_filter("length", length);
    env.add_filter("lines", lines);
    env.add_filter("list", list);
    env.add_filter("lower", lower);
    env.add_filter("map", map);
    env.add_filter("max", max);
    env.add_filter("min", min);
    env.add_filter("pprint", pprint);
    env.add_filter("reject", reject);
    env.add_filter("rejectattr", rejectattr);
    env.add_filter("replace", replace);
    env.add_filter("reverse", reverse);
    env.add_filter("round", round);
    env.add_filter("select", select);
    env.add_filter("selectattr", selectattr);
    env.add_filter("slice", slice);
    env.add_filter("sort", sort);
    env.add_filter("split", split);
    env.add_filter("string", string);
    env.add_filter("tojson", tojson);
    env.add_filter("trim", trim);
    env.add_filter("unique", unique);
    env.add_filter("upper", unique);   
}

fn register_functions(env: &mut minijinja::Environment) {
    use minijinja::functions::*;

    env.add_function("debug", debug);
    env.add_function("dict", dict);
    env.add_function("namespace", namespace);
    env.add_function("range", range);
    env.add_function("now", || Value::from_object(DateTime::now()));
}

fn register_tests(env: &mut minijinja::Environment) {
    use minijinja::tests::*;

    env.add_test("boolean", is_boolean);
    env.add_test("defined", is_defined);
    env.add_test("divisibleby", is_divisibleby);
    env.add_test("endingwith", is_endingwith);
    env.add_test("eq", is_eq);
    env.add_test("even", is_even);
    env.add_test("false", is_false);
    env.add_test("filter", is_filter);
    env.add_test("float", is_float);
    env.add_test("ge", is_ge);
    env.add_test("gt", is_gt);
    env.add_test("in", is_in);
    env.add_test("integer", is_integer);
    env.add_test("iterable", is_iterable);
    env.add_test("le", is_le);
    env.add_test("lower", is_lower);
    env.add_test("lt", is_lt);
    env.add_test("mapping", is_mapping);
    env.add_test("ne", is_ne);
    env.add_test("none", is_none);
    env.add_test("number", is_number);
    env.add_test("odd", is_odd);
    env.add_test("sequence", is_sequence);
    env.add_test("startingwith", is_startingwith);
    env.add_test("string", is_string);
    env.add_test("test", is_test);
    env.add_test("true", is_true);
    env.add_test("undefined", is_undefined);
    env.add_test("upper", is_upper);
}
