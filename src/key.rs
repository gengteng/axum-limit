use crate::Key;
use http::{Method, Uri, Version};

impl Key for Uri {
    type Extractor = Uri;

    fn from_extractor(extractor: &Self::Extractor) -> Self {
        extractor.clone()
    }
}

impl Key for Method {
    type Extractor = Method;

    fn from_extractor(extractor: &Self::Extractor) -> Self {
        extractor.clone()
    }
}

impl Key for Version {
    type Extractor = Version;

    fn from_extractor(extractor: &Self::Extractor) -> Self {
        *extractor
    }
}

macro_rules! impl_key_for_tuple {
    ($($name:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($name),+> Key for ($($name,)+)
        where
            $($name: Key,)+
        {
            type Extractor = ($($name::Extractor,)+);

            fn from_extractor(($($name,)+): &Self::Extractor) -> Self {
                ($($name::from_extractor($name),)+)
            }
        }
    }
}

impl_key_for_tuple!(T0);
impl_key_for_tuple!(T0, T1);
impl_key_for_tuple!(T0, T1, T2);
impl_key_for_tuple!(T0, T1, T2, T3);
impl_key_for_tuple!(T0, T1, T2, T3, T4);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5, T6);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5, T6, T7);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_key_for_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
