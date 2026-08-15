/// Identity of a handler.
pub mod handler_id {
    use std::any::TypeId;
    use std::fmt;

    /// Each handler declares a zero-sized marker type and gets a `HandlerId`
    /// from `HandlerId::of::<MarkerType>()`. The type's `TypeId` is the actual
    /// discriminator, so external crates can define their own handlers
    /// without coordinating IDs with anyone.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub struct HandlerId {
        type_id: TypeId,
        /// Human-readable name, captured for debugging; not used for identity.
        name: &'static str,
    }

    impl HandlerId {
        pub fn of<T: 'static>() -> Self {
            Self {
                type_id: TypeId::of::<T>(),
                name: std::any::type_name::<T>(),
            }
        }

        pub fn type_id(&self) -> TypeId {
            self.type_id
        }

        pub fn name(&self) -> &'static str {
            self.name
        }
    }

    impl fmt::Debug for HandlerId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_tuple("HandlerId").field(&self.name).finish()
        }
    }

    impl fmt::Display for HandlerId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // Strip the crate path for readability when printing.
            let short = self.name.rsplit("::").next().unwrap_or(self.name);
            f.write_str(short)
        }
    }
}

/// What conceptual scope an attribute belongs to. Identity applies regardless
/// of handler; Launch covers the generic exec primitives; HandlerScoped is
/// only meaningful when that specific handler is active.
pub mod category {
    use crate::handler_id::HandlerId;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum AttributeCategory {
        Identity,
        Launch,
        HandlerScoped(HandlerId),
    }
}
