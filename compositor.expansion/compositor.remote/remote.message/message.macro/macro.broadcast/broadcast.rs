#[macro_export]
macro_rules! define_broadcasts {
    (
        master: $master_name:ident,
        packages: {
            $(
                $package_variant:ident {
                    namespace: $namespace:ident,
                    enum: $sub_enum_name:ident,
                    messages: {
                        $( $variant:ident($msg_type:ty); )*
                    }
                }
            ),* $(,)?
        }
    ) => {
        $(
            pub mod $namespace {
                use super::*;
                #[derive(Debug, Clone, PartialEq)]
                pub enum $sub_enum_name {
                    $( $variant($msg_type), )*
                }
                impl $sub_enum_name {
                    pub fn variant_name(&self) -> &'static str {
                        match self {
                            $( Self::$variant(_) => stringify!($variant), )*
                        }
                    }
                }
            }
        )*
        #[derive(Debug, Clone, PartialEq)]
        pub enum $master_name {
            $( $package_variant($namespace::$sub_enum_name), )*
        }
        impl $master_name {
            pub fn package_name(&self) -> &'static str {
                match self {
                    $( Self::$package_variant(_) => stringify!($package_variant), )*
                }
            }
        }
    };
}
