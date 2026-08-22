mod channels;
mod runtime;
mod watch;

pub(crate) use channels::DiskDelivery;
pub(crate) use channels::DiskWorkerChannels;
pub(crate) use channels::DiskWorkerMessage;
pub(crate) use runtime::start_disk_worker;

pub(super) fn take_worker_message(
    disk_worker_channels: &DiskWorkerChannels,
) -> Option<DiskWorkerMessage> {
    disk_worker_channels.take_message()
}
