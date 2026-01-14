use jaq_core::{Native, Error, Bind, Ctx};
use jaq_core::Exn;
use jaq_json::Val;
use regex::Regex;
use std::string::{String, ToString};
use std::vec::Vec;
use std::boxed::Box;
use std::rc::Rc;
use indexmap::IndexMap;
use foldhash::fast::RandomState;

// Helper to extract string from Val
fn get_string(val: &Val) -> Result<String, Error<Val>> {
    match val {
        Val::Str(s, _) => Ok(String::from_utf8_lossy(s.as_ref()).to_string()),
        _ => Err(Error::typ(val.clone(), "string")),
    }
}

// Regex implementation helpers
fn test_impl(input_val: Val, regex_val: Val) -> Box<dyn Iterator<Item = Result<Val, Exn<Val>>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };
    
    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from(e.to_string())))))),
    };

    Box::new(std::iter::once(Ok(Val::Bool(re.is_match(&input)))))
}

fn capture_impl(input_val: Val, regex_val: Val) -> Box<dyn Iterator<Item = Result<Val, Exn<Val>>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };

    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from(e.to_string())))))),
    };

    if let Some(caps) = re.captures(&input) {
         let mut obj = IndexMap::with_hasher(RandomState::default());
         
         for n in re.capture_names().flatten() {
             if let Some(m) = caps.name(n) {
                 obj.insert(Val::from(n.to_string()), Val::from(m.as_str().to_string()));
             }
         }
         return Box::new(std::iter::once(Ok(Val::obj(obj))));
    }

    Box::new(std::iter::empty())
}

fn match_impl(input_val: Val, regex_val: Val, global: bool) -> Box<dyn Iterator<Item = Result<Val, Exn<Val>>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };
    // println!("DEBUG: match_impl input='{}' regex='{:?}'", input, regex_val);

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };
    
    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from(e.to_string())))))),
    };

    let matches: Vec<Val> = re.captures_iter(&input).map(|caps| {
        let mut obj = IndexMap::with_hasher(RandomState::default());
        let m = caps.get(0).unwrap();
        obj.insert(Val::from("offset".to_string()), Val::from(m.start()));
        obj.insert(Val::from("length".to_string()), Val::from(m.end() - m.start()));
        obj.insert(Val::from("string".to_string()), Val::from(m.as_str().to_string()));
        
        let mut captures = Vec::new();
        for i in 1..caps.len() {
            if let Some(c) = caps.get(i) {
                let mut cap_obj = IndexMap::with_hasher(RandomState::default());
                cap_obj.insert(Val::from("offset".to_string()), Val::from(c.start()));
                cap_obj.insert(Val::from("length".to_string()), Val::from(c.end() - c.start()));
                cap_obj.insert(Val::from("string".to_string()), Val::from(c.as_str().to_string()));
                if let Some(name) = re.capture_names().nth(i).flatten() {
                    cap_obj.insert(Val::from("name".to_string()), Val::from(name.to_string()));
                }
                captures.push(Val::obj(cap_obj));
            } else {
                captures.push(Val::Null);
            }
        }
        obj.insert(Val::from("captures".to_string()), Val::Arr(Rc::new(captures)));
        Val::obj(obj)
    }).collect();

    if global {
        Box::new(matches.into_iter().map(Ok))
    } else {
        Box::new(matches.into_iter().take(1).map(Ok))
    }
}

fn sub_impl(input_val: Val, regex_val: Val, repl_val: Val, global: bool) -> Box<dyn Iterator<Item = Result<Val, Exn<Val>>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };
    
    let replacement = match get_string(&repl_val) {
         Ok(s) => s,
         Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };

    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from(e.to_string())))))),
    };

    let result = if global {
        re.replace_all(&input, replacement.as_str())
    } else {
        re.replace(&input, replacement.as_str())
    };

    Box::new(std::iter::once(Ok(Val::from(result.to_string()))))
}

fn splits_impl(input_val: Val, regex_val: Val) -> Box<dyn Iterator<Item = Result<Val, Exn<Val>>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(e)))),
    };
    
    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from(e.to_string())))))),
    };

    let parts: Vec<Val> = re.split(&input).map(|s| Val::from(s.to_string())).collect();
    Box::new(parts.into_iter().map(Ok))
}

pub fn regex_natives() -> impl Iterator<Item = (&'static str, Box<[Bind]>, Native<jaq_core::data::JustLut<Val>>)> {
    vec![
        ("_native_test", Box::new([]) as Box<[Bind]>, Native::new(|(_, val): (Ctx<jaq_core::data::JustLut<Val>>, Val)| {
             let arr = match val {
                 Val::Arr(ref a) => a,
                 _ => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected array input".to_string())))))) as Box<dyn Iterator<Item = _>>,
             };
             if arr.len() < 2 {
                 return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected 2 args for test".to_string()))))));
             }
             test_impl(arr[0].clone(), arr[1].clone())
        })),
        ("_native_capture", Box::new([]) as Box<[Bind]>, Native::new(|(_, val): (Ctx<jaq_core::data::JustLut<Val>>, Val)| {
             let arr = match val {
                 Val::Arr(ref a) => a,
                 _ => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected array input".to_string())))))) as Box<dyn Iterator<Item = _>>,
             };
             if arr.len() < 2 {
                 return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected 2 args for capture".to_string()))))));
             }
             capture_impl(arr[0].clone(), arr[1].clone())
        })),
        ("_native_match", Box::new([]) as Box<[Bind]>, Native::new(|(_, val): (Ctx<jaq_core::data::JustLut<Val>>, Val)| {
             let arr = match val {
                 Val::Arr(ref a) => a,
                 _ => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected array input".to_string())))))) as Box<dyn Iterator<Item = _>>,
             };
             if arr.len() < 2 {
                 return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected 2 args for match".to_string()))))));
             }
             match_impl(arr[0].clone(), arr[1].clone(), true) // global matches for 'match'
        })),
        ("_native_splits", Box::new([]) as Box<[Bind]>, Native::new(|(_, val): (Ctx<jaq_core::data::JustLut<Val>>, Val)| {
             let arr = match val {
                 Val::Arr(ref a) => a,
                 _ => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected array input".to_string())))))) as Box<dyn Iterator<Item = _>>,
             };
             if arr.len() < 2 {
                 return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected 2 args for splits".to_string()))))));
             }
             splits_impl(arr[0].clone(), arr[1].clone())
        })),
        ("_native_sub", Box::new([]) as Box<[Bind]>, Native::new(|(_, val): (Ctx<jaq_core::data::JustLut<Val>>, Val)| {
             let arr = match val {
                 Val::Arr(ref a) => a,
                 _ => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected array input".to_string())))))) as Box<dyn Iterator<Item = _>>,
             };
             if arr.len() < 3 {
                 return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected 3 args for sub".to_string()))))));
             }
             sub_impl(arr[0].clone(), arr[1].clone(), arr[2].clone(), false)
        })),
        ("_native_gsub", Box::new([]) as Box<[Bind]>, Native::new(|(_, val): (Ctx<jaq_core::data::JustLut<Val>>, Val)| {
             let arr = match val {
                 Val::Arr(ref a) => a,
                 _ => return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected array input".to_string())))))) as Box<dyn Iterator<Item = _>>,
             };
             if arr.len() < 3 {
                 return Box::new(std::iter::once(Err(Exn::from(Error::new(Val::from("Expected 3 args for gsub".to_string()))))));
             }
             sub_impl(arr[0].clone(), arr[1].clone(), arr[2].clone(), true)
        })),
    ].into_iter()
}
