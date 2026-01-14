use jaq_interpret::{Native, Val, Error, FilterT};
use jaq_interpret::error::Type;
use regex::Regex;
use std::rc::Rc;
use indexmap::IndexMap;
use ahash::RandomState;

// Define the iterator type for our generic return
// jaq natives return Box<dyn Iterator<Item = Result<Val, Error>>>

pub fn regex_natives() -> impl Iterator<Item = (String, usize, Native)> {
    vec![
        ("_native_test".to_string(), 1, Native::new(|args: jaq_interpret::Args<'_, Val>, (ctx, val)| {
             // Argument 0 is the regex filter
             let regex_filter = args.get(0); 
             // Evaluate filter on current input
             let regex_vals = regex_filter.run((ctx.clone(), val.clone()));
             
             // For each yield of the regex filter, we test input
             Box::new(regex_vals.flat_map(move |r| -> Box<dyn Iterator<Item = Result<Val, Error>>> {
                 match r {
                     Ok(v) => test_impl(val.clone(), v), // val is the input string, v is the regex pattern
                     Err(e) => Box::new(std::iter::once(Err(e))),
                 }
             }))
        })),
        ("_native_capture".to_string(), 1, Native::new(|args: jaq_interpret::Args<'_, Val>, (ctx, val)| {
             let regex_filter = args.get(0);
             let regex_vals = regex_filter.run((ctx.clone(), val.clone()));
             
             Box::new(regex_vals.flat_map(move |r| -> Box<dyn Iterator<Item = Result<Val, Error>>> {
                 match r {
                     Ok(v) => capture_impl(val.clone(), v),
                     Err(e) => Box::new(std::iter::once(Err(e))),
                 }
             }))
        })),
        ("_native_sub".to_string(), 2, Native::new(|args: jaq_interpret::Args<'_, Val>, (ctx, val)| {
             let regex_filter = args.get(0);
             // We clone args to use inside the closure, allowing us to get repl_filter freshly
             let args_clone = args;
             
             let ctx_clone = ctx.clone();
             let val_clone = val.clone(); // Input string
             
             let regex_vals = regex_filter.run((ctx.clone(), val.clone()));
             
             Box::new(regex_vals.flat_map(move |r_res: Result<Val, Error>| -> Box<dyn Iterator<Item = Result<Val, Error>>> {
                 match r_res {
                     Ok(r_val) => {
                         // Get repl_filter from cloned args
                         let repl_filter = args_clone.get(1);
                         
                         let repl_vals = repl_filter.run((ctx_clone.clone(), val_clone.clone())); 
                         let input = val_clone.clone();
                         let r_val = r_val.clone();
                         Box::new(repl_vals.flat_map(move |rep_res: Result<Val, Error>| -> Box<dyn Iterator<Item = Result<Val, Error>>> {
                             match rep_res {
                                 Ok(rep_val) => sub_impl(input.clone(), r_val.clone(), rep_val, false),
                                 Err(e) => Box::new(std::iter::once(Err(e))),
                             }
                         }))
                     },
                     Err(e) => Box::new(std::iter::once(Err(e))),
                 }
             }))
        })),
        ("_native_gsub".to_string(), 2, Native::new(|args: jaq_interpret::Args<'_, Val>, (ctx, val)| {
             let regex_filter = args.get(0);
             let args_clone = args;
             
             let ctx_clone = ctx.clone();
             let val_clone = val.clone(); 
             let regex_vals = regex_filter.run((ctx.clone(), val.clone()));
             
             Box::new(regex_vals.flat_map(move |r_res: Result<Val, Error>| -> Box<dyn Iterator<Item = Result<Val, Error>>> {
                 match r_res {
                     Ok(r_val) => {
                         let repl_filter = args_clone.get(1);
                         let repl_vals = repl_filter.run((ctx_clone.clone(), val_clone.clone())); 
                         let input = val_clone.clone();
                         let r_val = r_val.clone();
                         Box::new(repl_vals.flat_map(move |rep_res: Result<Val, Error>| -> Box<dyn Iterator<Item = Result<Val, Error>>> {
                             match rep_res {
                                 Ok(rep_val) => sub_impl(input.clone(), r_val.clone(), rep_val, true),
                                 Err(e) => Box::new(std::iter::once(Err(e))),
                             }
                         }))
                     },
                     Err(e) => Box::new(std::iter::once(Err(e))),
                 }
             }))
        })),
    ].into_iter()
}

fn get_string(val: &Val) -> Result<String, Error> {
    match val {
        Val::Str(s) => Ok(s.to_string()),
        _ => Err(Error::Type(val.clone(), Type::Str)),
    }
}

fn test_impl(input_val: Val, regex_val: Val) -> Box<dyn Iterator<Item = Result<Val, Error>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(e))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(e))),
    };
    
    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Error::str(e.to_string())))),
    };

    Box::new(std::iter::once(Ok(Val::Bool(re.is_match(&input)))))
}

fn capture_impl(input_val: Val, regex_val: Val) -> Box<dyn Iterator<Item = Result<Val, Error>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(e))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(e))),
    };

    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Error::str(e.to_string())))),
    };

    if let Some(caps) = re.captures(&input) {
         // Create IndexMap with ahash hasher
         let mut obj: IndexMap<Rc<String>, Val, RandomState> = IndexMap::with_hasher(RandomState::new());
         for n in re.capture_names().flatten() {
             if let Some(m) = caps.name(n) {
                 obj.insert(Rc::new(n.to_string()), Val::Str(Rc::new(m.as_str().to_string())));
             }
         }
         return Box::new(std::iter::once(Ok(Val::Obj(Rc::new(obj)))));
    }

    Box::new(std::iter::empty())
}

fn sub_impl(input_val: Val, regex_val: Val, repl_val: Val, global: bool) -> Box<dyn Iterator<Item = Result<Val, Error>>> {
    let input = match get_string(&input_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(e))),
    };

    let regex_str = match get_string(&regex_val) {
        Ok(s) => s,
        Err(e) => return Box::new(std::iter::once(Err(e))),
    };
    
    let replacement = match get_string(&repl_val) {
         Ok(s) => s,
         Err(e) => return Box::new(std::iter::once(Err(e))),
    };

    let re = match Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return Box::new(std::iter::once(Err(Error::str(e.to_string())))),
    };

    // Note: rust regex replacement syntax might differ slightly from jq's (PCRE vs safe regex)
    // jq uses named captures in replacement string? 
    // For now we assume standard rust regex replacement behavior.
    
    let result = if global {
        re.replace_all(&input, replacement.as_str())
    } else {
        re.replace(&input, replacement.as_str())
    };

    Box::new(std::iter::once(Ok(Val::Str(Rc::new(result.to_string())))))
}
