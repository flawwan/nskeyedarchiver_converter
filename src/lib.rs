pub use plist;
use plist::{Dictionary, Value};
use thiserror::Error;

const ARCHIVER: &str = "NSKeyedArchiver";
const ARCHIVER_VERSION: u64 = 100000;

const ARCHIVER_KEY_NAME: &str = "$archiver";
const TOP_KEY_NAME: &str = "$top";
const OBJECTS_KEY_NAME: &str = "$objects";
const VERSION_KEY_NAME: &str = "$version";
const NULL_OBJECT_REFERENCE_NAME: &str = "$null";

#[derive(Error, Debug)]
pub enum NSKeyedUnarchiverError {
    #[error("Plist error: {0}")]
    PlistError(String),
    #[error("Expected '{0}' key to be a type of '{1}'")]
    WrongValueType(&'static str, &'static str),
    #[error("Missing '{0}' header key")]
    MissingHeaderKey(&'static str),
    #[error("Unsupported archiver. Only '{ARCHIVER}' is supported")]
    UnsupportedArchiver,
    #[error("Unsupported archiver version. Only '{ARCHIVER_VERSION}' is supported")]
    UnsupportedArchiverVersion,
    #[error("Invalid object reference ({0}). The data may be corrupt.")]
    InvalidObjectReference(u64),
    #[error("Invalid object encoding ({0}). The data may be corrupt.")]
    InvalidObjectEncoding(u64),
    #[error("Invalid class reference ({0}). The data may be corrupt.")]
    InvalidClassReference(String),
    #[error("Expected uid value for key {0}")]
    ExpectedUIDValue(String)
}

impl From<plist::Error> for NSKeyedUnarchiverError {
    fn from(value: plist::Error) -> Self {
        Self::PlistError(
            match value.is_io() {
                true => value.into_io().unwrap().to_string(),
                false => value.to_string()
            }
        )
    }
}

macro_rules! uid {
    ($name:ident, $key:expr) => {
        match ($name.as_uid()) {
            Some(u) => u,
            None => {
                return Err(NSKeyedUnarchiverError::ExpectedUIDValue($key))
            }
        }
    };
}

pub struct NSKeyedUnarchiver {
    objects: Vec<Value>,
    top: Dictionary,
}

impl NSKeyedUnarchiver {
    pub fn new(plist: Value) -> Result<Self, NSKeyedUnarchiverError> {
        let Some(mut dict) = plist.into_dictionary() else {
            return Err(NSKeyedUnarchiverError::WrongValueType("root", "Dictionary"));
        };

        // Check $archiver key
        let archiver_key = Self::get_header_key(&mut dict, ARCHIVER_KEY_NAME)?;
        let Some(archiver_str) = archiver_key.as_string() else {
            return Err(NSKeyedUnarchiverError::WrongValueType(
                ARCHIVER_KEY_NAME,
                "String",
            ));
        };

        if archiver_str != ARCHIVER {
            return Err(NSKeyedUnarchiverError::UnsupportedArchiver);
        }

        // Check $version key
        let version_key = Self::get_header_key(&mut dict, VERSION_KEY_NAME)?;
        let Some(version_num) = version_key.as_unsigned_integer() else {
            return Err(NSKeyedUnarchiverError::WrongValueType(
                VERSION_KEY_NAME,
                "Number",
            ));
        };

        if version_num != ARCHIVER_VERSION {
            return Err(NSKeyedUnarchiverError::UnsupportedArchiverVersion);
        }

        // Check $top key
        let top_key = Self::get_header_key(&mut dict, TOP_KEY_NAME)?;
        let Some(top) = top_key.to_owned().into_dictionary() else {
            return Err(NSKeyedUnarchiverError::WrongValueType(
                TOP_KEY_NAME,
                "Dictionary",
            ));
        };

        // Check $objects key
        let objects_key = Self::get_header_key(&mut dict, OBJECTS_KEY_NAME)?;
        let Some(objects) = objects_key.into_array() else {
            return Err(NSKeyedUnarchiverError::WrongValueType(
                OBJECTS_KEY_NAME,
                "Array",
            ));
        };

        Ok(Self {
            objects,
            top,
        })
    }
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, NSKeyedUnarchiverError> {
        let val: plist::Value = plist::from_file(path)?;
        Self::new(val)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NSKeyedUnarchiverError> {
        let val: plist::Value = plist::from_bytes(bytes)?;
        Self::new(val)
    }

    pub fn from_reader<R: std::io::Read + std::io::Seek>(
        reader: R,
    ) -> Result<Self, NSKeyedUnarchiverError> {
        let val: plist::Value = plist::from_reader(reader)?;
        Self::new(val)
    }

    pub fn decode(&mut self) -> Result<Value, NSKeyedUnarchiverError> {
        let mut dict = Dictionary::new();
        for (key, value) in &self.top {
            let uid = uid!(value, key.to_string());
            println!("-- TOP: {key} (uid={}) --", uid.get());
            let Some(value) = self.decode_object(&uid.clone())? else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid.get()));
            };
            dict.insert(
                key.clone(),
                value,
            );
        }
        Ok(Value::Dictionary(dict))
    }

    fn get_header_key(dict: &mut Dictionary, key: &'static str) -> Result<Value, NSKeyedUnarchiverError> {
        let Some(objects_value) = dict.remove(key) else {
            return Err(NSKeyedUnarchiverError::MissingHeaderKey(key));
        };
        Ok(objects_value)
    }

    fn decode_object(&self, uid: &plist::Uid) -> Result<Option<Value>, NSKeyedUnarchiverError> {
        let object_ref = uid.get();

        if object_ref == 0 {
            println!("uid == 0");
            return Ok(None);
        }

        let Some(dereferenced_object) = self.objects.get(object_ref as usize) else {
            return Err(NSKeyedUnarchiverError::InvalidObjectReference(object_ref));
        };

        if let Some(s) = dereferenced_object.as_string() {
            if s == NULL_OBJECT_REFERENCE_NAME {
                //return None;
            }
        }

        let mut result = None;
        if Self::is_container(dereferenced_object) {
            println!("decode_object: dereferenced_object (uid={object_ref}) is a container");
            let Some(dict) = dereferenced_object.as_dictionary() else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(object_ref));
            };

            let Some(class_reference_val) = dict.get("$class") else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(object_ref));
            };
            let Some(class_reference) = class_reference_val.as_uid() else {
                return Err(NSKeyedUnarchiverError::InvalidClassReference(format!(
                    "{:?}",
                    class_reference_val
                )));
            };

            let class_names = self.get_class_names(class_reference)?;
            let mut found = false;
            for name in class_names {
                if found {
                    break;
                }
                result = match name {
                    "NSMutableDictionary" | "NSDictionary" => {
                        found = true;
                        println!("decode_object: Decoding dictionary (uid={})", object_ref);
                        Some(self.decode_dict(object_ref, dict)?)
                    }
                    "NSMutableArray" | "NSArray" => {
                        found = true;
                        println!("decode_object: Decoding array (uid={})", object_ref);
                        Some(self.decode_array(object_ref, dict)?)
                    }
                    _ => {
                        found = true;
                        println!("decode_object: Decoding basic class (uid={})", object_ref);
                        Some(self.decode_custom_class(object_ref, dict)?)
                    }
                };
            }
            Ok(result)
        } else {
            println!("decode_object: dereferenced_object (uid={object_ref}) is NOT a container. Return {:?}", dereferenced_object);
            Ok(Some(dereferenced_object.clone()))
        }
    }

    fn get_class_names(&self, uid: &plist::Uid) -> Result<Vec<&str>, NSKeyedUnarchiverError> {
        println!("get_class_names: uid = {}", uid.get());

        let Some(obj) = self.objects.get(uid.get() as usize) else {
            return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid.get()));
        };

        let Some(names) = obj
            .as_dictionary()
            .and_then(|dict| dict.get("$classes").and_then(|classes| classes.as_array())) else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid.get()));
            };

        let mut vec_of_names = Vec::new();
        for name in names {
            let Some(name) = name.as_string() else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid.get()));
            };
            vec_of_names.push(name);
        }
        Ok(vec_of_names)
    }

    fn is_container(val: &Value) -> bool {
        let Some(dict) = val.as_dictionary() else {
            return false;
        };
        if let Some(cls) = dict.get("$class") {
            cls.as_uid().is_some()
        } else {
            false
        }
    }

    fn decode_custom_class(&self, uid: u64, val: &Dictionary) -> Result<Value, NSKeyedUnarchiverError> {
        let mut class_dict = Dictionary::new();
        for (key, value) in val {
            if key == "$class" {
                println!("{:?}", value);
                let Some(classes_obj) = self.decode_object(uid!(value, key.to_string()))? else {
                    return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
                };
                let Some(classes) = classes_obj
                    .as_dictionary()
                    .and_then(|dict| dict.get("$classes")) else {
                        return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
                    };
                class_dict.insert("$classes".to_string(), classes.clone());
                continue;
            }

            let decoded_value = match value.as_uid() {
                Some(v) => self.decode_object(v)?,
                None => Some(value.clone()),
            };
            if let Some(v) = decoded_value {
                class_dict.insert(key.clone(), v);
            } else {
                println!("decode_basic_class: Skipping an empty key-value pair");
            }
        }
        Ok(Value::Dictionary(class_dict))
    }

    fn decode_array(&self, uid: u64, val: &Dictionary) -> Result<Value, NSKeyedUnarchiverError> {
        println!("decode_array: {:?}", val);
        let Some(raw_object) = val.get("NS.objects").and_then(|objs| objs.as_array()) else {
            return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
        };
        let mut array: Vec<Value> = Vec::with_capacity(raw_object.len());
        for element in raw_object {
            let decoded_value = self.decode_object(uid!(element, "NS.objects".to_string()))?;
            if let Some(v) = decoded_value {
                array.push(v);
            } else {
                println!("decode_array: Skipping an empty key-value pair");
            }
        }
        Ok(Value::Array(array))
    }

    fn decode_dict(&self, uid: u64, val: &Dictionary) -> Result<Value, NSKeyedUnarchiverError> {
        let Some(keys) = val.get("NS.keys").and_then(|keys| keys.as_array()) else {
            return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
        };
        let Some(values) = val.get("NS.objects").and_then(|objs| objs.as_array()) else {
            return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
        };
        println!("Decode dict, keys: {:?}", keys);
        println!("Decode dict, values: {:?}", values);

        // Decode keys and values
        let mut decoded_keys = Vec::with_capacity(keys.len());
        let mut decoded_values = Vec::with_capacity(values.len());
        for key in keys {
            let Some(decoded_key) = self.decode_object(uid!(key, "NS.keys".to_string()))? else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
            };
            decoded_keys.push(decoded_key);
        }
        for value in values {
            let Some(decoded_value) = self.decode_object(uid!(value, "NS.objects".to_string()))? else {
                return Err(NSKeyedUnarchiverError::InvalidObjectEncoding(uid));
            };
            decoded_values.push(decoded_value);
        }

        println!("decode_dict: decoded_keys = {:?}", decoded_keys);
        println!("decode_dict: decoded_values = {:?}", decoded_keys);

        // A dictionary key can be a number, a string or a custom object.
        // So we rather make an a array of dictionaries
        let mut array_of_dicts = Vec::with_capacity(decoded_keys.len());
        while !decoded_keys.is_empty() {
            let mut dict: Dictionary = Dictionary::new();
            dict.insert("key".to_string(), decoded_keys.remove(0));
            dict.insert("value".to_string(), decoded_values.remove(0));
            array_of_dicts.push(Value::Dictionary(dict));
        }

        Ok(Value::Array(array_of_dicts))
    }
}
