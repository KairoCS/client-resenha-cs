// gsi/ — Game State Integration: a ponte com o CS2.
//
//   instalacao.rs  põe o .cfg na pasta do jogo (é o que liga o GSI)
//   servidor.rs    recebe os POST do CS2 em 127.0.0.1 e filtra o que sai daqui
mod instalacao;
mod servidor;

pub use instalacao::install_cfg;
pub use servidor::run_server;
