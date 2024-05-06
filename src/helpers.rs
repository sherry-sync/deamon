use std::collections::{BTreeMap, HashMap};
use serde::{Serialize, Serializer};

pub fn ordered_map<S, K: Ord + Serialize, V: Serialize>(
    value: &HashMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
{
    let ordered: BTreeMap<_, _> = value.iter().collect();
    ordered.serialize(serializer)
}

pub fn str_err_prefix<T: ToString + 'static>(prefix: &'static str) -> impl Fn(T) -> String {
    move |e| {
        let msg = format!("{}: {}", prefix, e.to_string());
        println!("{}", msg);
        log::info!("{}", msg);
        msg
    }
}
