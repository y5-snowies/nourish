#[macro_export]
macro_rules! define {
    (
        server: $server_struct:ident,
        dispatch: $dispatch_method:ident,
        master_enum: $master_enum_name:ident,
        services: {
            $(
                $service_variant:ident {
                    trait: $service_trait:path,
                    namespace: $namespace:ident,
                    enum: $sub_enum_name:ident,
                    handler_trait: $handler_trait:ident,
                    methods: {
                        $( $fn_name:ident => $variant:ident($req:ty) -> $res:ty; )*
                    }
                }
            ),* $(,)?
        }
    ) => {
        $(
            pub trait $handler_trait<T> {
                $(
                    fn $fn_name(&mut self, request: $req, state: &mut T) -> $res;
                )*
            }
        )*
        $(
            pub mod $namespace {
                use super::*;
                #[derive(Debug)]
                pub enum $sub_enum_name {
                    $(
                        $variant($req, tokio::sync::oneshot::Sender<$res>),
                    )*
                }
                impl $sub_enum_name {
                    pub fn execute<T, H: super::$handler_trait<T>>(self, handler: &mut H, state: &mut T) {
                        match self {
                            $(
                                Self::$variant(req, tx) => {
                                    let response = handler.$fn_name(req, state);
                                    let _ = tx.send(response);
                                }
                            )*
                        }
                    }
                }
            }
        )*
        #[derive(Debug)]
        pub enum $master_enum_name {
            $(
                $service_variant($namespace::$sub_enum_name),
            )*
        }
        impl $master_enum_name {
            pub fn execute<T, H>(self, handler: &mut H, state: &mut T)
            where
                $( H: $handler_trait<T> ),*
            {
                match self {
                    $(
                        Self::$service_variant(inner_msg) => inner_msg.execute(handler, state),
                    )*
                }
            }
        }
        $(
            #[tonic::async_trait]
            impl $service_trait for $server_struct {
                $(
                    async fn $fn_name(
                        &self,
                        request: tonic::Request<$req>,
                    ) -> Result<tonic::Response<$res>, tonic::Status> {
                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                        let inner_msg = $namespace::$sub_enum_name::$variant(request.into_inner(), reply_tx);
                        let master_msg = $master_enum_name::$service_variant(inner_msg);
                        self.$dispatch_method(master_msg)?;
                        match reply_rx.await {
                            Ok(response) => Ok(tonic::Response::new(response)),
                            Err(_) => Err(tonic::Status::internal("Compositor dropped the request")),
                        }
                    }
                )*
            }
        )*
    };
}
