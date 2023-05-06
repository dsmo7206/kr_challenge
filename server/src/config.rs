use envconfig::Envconfig;

#[derive(Envconfig)]
pub struct Config {
    #[envconfig(from = "GRPC_PORT", default = "50051")]
    pub grpc_port: u16,

    #[envconfig(from = "REST_PORT", default = "3000")]
    pub rest_port: u16,
}
