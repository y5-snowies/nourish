use crate::bind;
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};

pub struct Message {
    pub Value: Service,
}

#[derive(Clone)]
pub struct MessageClientReceiver {
    // The macro expects this field to exist
    pub calloop_tx: smithay::reexports::calloop::channel::Sender<Message>,
}

impl MessageClientReceiver {
    // We define the delegation method here
    fn send_to_calloop(&self, msg: Service) -> Result<(), tonic::Status> {
        self.calloop_tx
            .send(Message { Value: msg })
            .map_err(|_| tonic::Status::internal("Calloop event loop is unavailable"))
    }
}



// Invoke it ONCE in your project
compositor_remote_message_macro_base::define! {
    server: MessageClientReceiver,
    dispatch: send_to_calloop,
    master_enum: Service, // Your global enum name

    services: {
        Navigator { //
            trait: bind::navigator::navigator_server::Navigator,
            namespace: navigator,
            // server_wrapper: bind::navigator::navigator_server::NavigatorServer,
            enum: Navigator,
            handler_trait: NavigatorService,
            methods: {
                travel => Travel(bind::navigator::Travel) -> bind::navigator::TravelResponse;
            }
        },
        Selection { //
            trait: bind::selection::selection_server::Selection,
            namespace: selection,
            enum: Selection,
            handler_trait: SelectionService,
            methods: {
                layout => Layout(bind::selection::Layout) -> bind::selection::LayoutResponse;
                fit_aspect => FitAspect(bind::selection::FitAspect) -> bind::selection::FitAspectResponse;
            }
        },
        Shader { //
            trait: bind::shader::shader_server::Shader,
            namespace: shader,
            enum: Shader,
            handler_trait: ShaderService,
            methods: {
                active => Active(bind::shader::ActiveRequest) -> bind::shader::ActiveResponse;
                list => List(bind::shader::ListRequest) -> bind::shader::ListResponse;
                activate => Activate(bind::shader::ActivateRequest) -> bind::shader::ActivateResponse;
                reload => Reload(bind::shader::ReloadRequest) -> bind::shader::ReloadResponse;
            }
        },
        Debug { //
            trait: bind::debug::debug_server::Debug,
            namespace: debug,
            // server_wrapper: bind::debug::debug_server::DebugServer,
            enum: Debug,
            handler_trait: DebugService,
            methods: {
                debug => Debug(bind::debug::Request) -> bind::debug::Response;
                numeric => Numeric(bind::debug::RequestNumeric) -> bind::debug::ResponseNumeric;
            }
        },
    }
}


