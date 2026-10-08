//! Moteur de simulation réseau NETIV.
//!
//! Simulation à événements discrets, mono-thread et déterministe.
//! Ce crate ne dépend d'aucune interface utilisateur ni d'aucun runtime
//! asynchrone (voir `SPEC_ARCHITECTURE.md`).

pub mod acl;
pub mod addr;
pub mod device;
pub mod error;
pub mod fault;
pub mod ids;
pub mod ip_node;
pub mod packet;
pub mod queue;
pub mod render;
pub mod rng;
pub mod route;
pub mod service;
pub mod sim;
pub mod switch;
pub mod time;
pub mod trace;

pub use addr::{Ipv4Net, MacAddr};
pub use device::{Command, Device, DeviceKind, Interface};
pub use error::NetivError;
pub use fault::{ActiveFault, Fault, Target};
pub use ids::{DeviceId, IfaceId, LinkId, TimerId, TraceId};
pub use rng::SimRng;
pub use route::StaticRoute;
pub use sim::{Event, Fired, Link, LinkParams, Simulation};
pub use time::SimTime;
pub use trace::{TraceEvent, TraceKind, TraceLog};
