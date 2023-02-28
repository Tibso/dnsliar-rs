use crate::redis_mod;
use crate::resolver_mod;

use smallvec::smallvec;
use trust_dns_resolver::Name;
use trust_dns_resolver::{
    AsyncResolver,
    name_server::{GenericConnection, GenericConnectionProvider, TokioRuntime}
};
use trust_dns_server::{
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
    proto::op::{Header, ResponseCode, OpCode, MessageType},
    authority::MessageResponseBuilder
};
use trust_dns_proto::rr::{
    Record,
    RData, 
    RecordType,
    rdata::{TXT, SRV}
};
use tracing::error;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use smallvec::{
    SmallVec,
    ToSmallVec
};

#[derive(thiserror::Error, Debug)]
pub enum CustomError {
    #[error("Invalid OpCode {0:}")]
    InvalidOpCode(OpCode),

    #[error("Invalid MessageType {0:}")]
    InvalidMessageType(MessageType),

    #[error("Redis Error {0:}")]
    RedisError(redis::RedisError),

    #[error("IO error: {0:}")]
    IOError(std::io::Error),

    #[error("Resolver error: {0:}")]
    ResolverError(trust_dns_resolver::error::ResolveError)
}

#[async_trait::async_trait]
impl RequestHandler for Handler {
    async fn handle_request <R: ResponseHandler> (
        &self,
        request: &Request,
        response: R
    )
    -> ResponseInfo {
        match self.do_handle_request(request, response).await {
            Ok(info) => info,
            Err(error) => {
                error!("RequestHandler error: {}", error);
                let mut header = Header::new();
                header.set_response_code(ResponseCode::ServFail);
                header.into()
            }
        }
    }
}

pub struct Handler {
    pub redis_manager: redis::aio::ConnectionManager,
    pub matchclasses: Vec<String>,
    pub resolver: AsyncResolver<GenericConnection, GenericConnectionProvider<TokioRuntime>>
}
impl Handler {
    async fn do_handle_request <R: ResponseHandler> (
        &self,
        request: &Request,
        response: R
    )
    -> Result<ResponseInfo, CustomError> {
        if request.op_code() != OpCode::Query {
            return Err(CustomError::InvalidOpCode(request.op_code()))
        }

        if request.message_type() != MessageType::Query {
            return Err(CustomError::InvalidMessageType(request.message_type()))
        }

        let domain_name = request.query().name().to_string().to_lowercase();
        let names = domain_name.split(".");

        let name_count = names.clone().count();
        let filter_5: [u8; 5] = [3, 4, 2, 5, 1];
        let mut order: SmallVec<[u8; 5]> = smallvec![];
        match name_count -1 {
            1 => order = smallvec![1],
            2 => order = smallvec![2, 1],
            3 => order = smallvec![3, 2, 1],
            4 => order = smallvec![3, 4, 2, 1],
            5 => order = filter_5.to_smallvec(),
            _ => {
                order.extend(1..name_count as u8);
                order = filter_5.to_smallvec();
                for index in 6..name_count {
                    order.push(index as u8)
                }
            }
        }

        let names: Vec<&str> = names.collect();
        for index in order {
            let mut domain_to_check = names[name_count - (index as usize)..name_count - 1].join(".");
            domain_to_check.push('.');

            for matchclass in &self.matchclasses {
                match redis_mod::exists(
                    &self.redis_manager,
                    format!("{}:{}", matchclass, domain_to_check),
                    request.src().is_ipv4()
                ).await {
                    Ok(ok) => {
                        if ok {
                            return self.should_lie(true, request, response).await
                        }
                    },
                    Err(error) => return Err(CustomError::RedisError(error))
                };
            }
        }

        return self.should_lie(false, request, response).await
    }

    async fn should_lie <R: ResponseHandler> (
        &self,
        should: bool,
        request: &Request,
        mut responder: R
    )
    -> Result<ResponseInfo, CustomError> {
        let builder = MessageResponseBuilder::from_message_request(request);
        let mut header = Header::response_from_request(request.header());
        header.set_authoritative(false);
        header.set_recursion_available(true);

        let answers: Vec<Record>;
        match should {
            false => (answers, header) = {
                match resolver_mod::get_answers(request.query(), header, self.resolver.clone()).await {
                    Ok(ok) => ok,
                    Err(_) => (vec![], header)
                }
            },
            true => answers = {
                let rdata = match request.query().query_type() {
                    RecordType::A => RData::A(Ipv4Addr::new(127, 0, 0, 1)),
                    RecordType::AAAA => RData::AAAA(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
                    RecordType::TXT => RData::TXT(TXT::new(vec!["127.0.0.1".to_string()])),
                    RecordType::SRV => RData::SRV(SRV::new(0, 0, 1053, Name::from_str("thib-deb.").unwrap())),
                    _ => todo!()
                };
                vec![Record::from_rdata(request.query().name().into(), 60, rdata)]
            }
        }

        let response = builder.build(header, answers.iter(), &[], &[], &[]);
        return match responder.send_response(response).await {
            Ok(ok) => Ok(ok),
            Err(error) => Err(CustomError::IOError(error))
        }
    }
}