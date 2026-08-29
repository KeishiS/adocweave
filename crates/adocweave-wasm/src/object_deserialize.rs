use std::fmt;
use std::marker::PhantomData;

use serde::de::value::MapAccessDeserializer;
use serde::de::{MapAccess, Visitor};

pub(crate) fn from_map<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct ObjectVisitor<T>(PhantomData<T>);

    impl<'de, T> Visitor<'de> for ObjectVisitor<T>
    where
        T: serde::Deserialize<'de>,
    {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object")
        }

        fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            T::deserialize(MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(ObjectVisitor(PhantomData))
}

macro_rules! serde_object {
    (
        #[cfg_attr($($type_cfg:tt)*)]
        #[derive($($derive:ident),+ $(,)?) ]
        #[wire(default, $($container:tt)*)]
        $visibility:vis struct $name:ident as $helper:ident {
            $($fields:tt)*
        }
    ) => {
        serde_object! {
            #[cfg_attr($($type_cfg)*)]
            #[derive($($derive),+)]
            #[wire($($container)*)]
            $visibility struct $name as $helper {
                $($fields)*
            }
        }
    };
    (
        #[cfg_attr($($type_cfg:tt)*)]
        #[derive($($derive:ident),+ $(,)?) ]
        #[wire($($container:tt)*)]
        $visibility:vis struct $name:ident as $helper:ident {
            $(
                $(#[cfg_attr($($field_cfg:tt)*)])?
                $(#[wire_field($wire_attribute:meta)])?
                pub $field:ident: $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[cfg_attr($($type_cfg)*)]
        #[derive($($derive),+, serde::Serialize)]
        #[serde($($container)*)]
        #[cfg_attr(feature = "ts-rs", ts(rename_all = "camelCase"))]
        $visibility struct $name {
            $(
                $(#[cfg_attr($($field_cfg)*)])?
                $(#[$wire_attribute])?
                pub $field: $field_type,
            )*
        }

        #[derive(serde::Deserialize)]
        #[serde($($container)*)]
        struct $helper {
            $(
                $(#[$wire_attribute])?
                $field: $field_type,
            )*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value: $helper = crate::object_deserialize::from_map(deserializer)?;
                Ok(Self {
                    $($field: value.$field,)*
                })
            }
        }
    };
}

pub(crate) use serde_object;

macro_rules! serde_object_serializable {
    (
        #[cfg_attr($($type_cfg:tt)*)]
        #[derive($($derive:tt)*)]
        #[wire($($container:tt)*)]
        $visibility:vis struct $name:ident as $helper:ident {
            $(
                $(#[cfg_attr($($field_cfg:tt)*)])?
                $(#[wire_field($wire_attribute:meta)])?
                pub $field:ident: $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[cfg_attr($($type_cfg)*)]
        #[derive($($derive)*)]
        #[serde($($container)*)]
        $visibility struct $name {
            $(
                $(#[cfg_attr($($field_cfg)*)])?
                $(#[$wire_attribute])?
                pub $field: $field_type,
            )*
        }

        #[derive(serde::Deserialize)]
        #[serde($($container)*)]
        struct $helper {
            $(
                $(#[$wire_attribute])?
                $field: $field_type,
            )*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value: $helper = crate::object_deserialize::from_map(deserializer)?;
                Ok(Self {
                    $($field: value.$field,)*
                })
            }
        }
    };
}

pub(crate) use serde_object_serializable;
