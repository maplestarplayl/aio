//! proactor.rs

use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use futures::task::{ArcWake, waker_ref};

use crate::net::AcceptFuture;
use crate::poller::Poller;

thread_local! {
    static PROACTOR: Proactor = Proactor::new().expect("failed to init")
}

type Task = Pin<Box<dyn Future<Output = ()> + 'static>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct TaskId(u64);

struct TaskState<T> {
    result: Option<T>,
    waker: Option<Waker>,
}

pub struct JoinHandle<T> {
    #[allow(dead_code)]
    task_id: TaskId,
    state: Rc<RefCell<TaskState<T>>>,
}

impl<T: 'static> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.borrow_mut();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// `Proactor` acts as both reactor and executor here
pub struct Proactor {
    poller: RefCell<Poller>,
    tasks: RefCell<HashMap<TaskId, Task>>,
    ready_queue: RefCell<VecDeque<TaskId>>,
}

struct TaskWaker {
    task_id: TaskId,
}

impl ArcWake for TaskWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        Proactor::with(|p| {
            p.ready_queue.borrow_mut().push_back(arc_self.task_id);
            p.poller.borrow().wakeup().expect("wakeup failed");
        });
    }
}

impl Proactor {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            poller: RefCell::new(Poller::new()?),
            tasks: RefCell::new(HashMap::new()),
            ready_queue: RefCell::new(VecDeque::new()),
        })
    }

    pub fn with<F, R>(f: F) -> R
    where
        F: FnOnce(&Proactor) -> R,
    {
        PROACTOR.with(f)
    }

    pub fn get_poller(&self) -> RefMut<'_, Poller> {
        self.poller.borrow_mut()
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let task_id = Proactor::new_task_id();
        let state = Rc::new(RefCell::new(TaskState {
            result: None,
            waker: None,
        }));

        let handle = JoinHandle {
            task_id,
            state: state.clone(),
        };

        let task = Box::pin(async move {
            let output = future.await;
            let mut state = state.borrow_mut();
            state.result = Some(output);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        });

        self.tasks.borrow_mut().insert(task_id, task);
        self.ready_queue.borrow_mut().push_back(task_id);

        handle
    }

    pub fn run(&self) -> io::Result<()> {
        while !self.tasks.borrow().is_empty() {
            let mut tasks_to_run = self.ready_queue.borrow_mut().split_off(0);
            

            while let Some(task_id) = tasks_to_run.pop_front() {
                let mut task = if let Some(task) = self.tasks.borrow_mut().remove(&task_id) {
                    task
                } else {
                    continue;
                };

                let task_waker = Arc::new(TaskWaker { task_id });
                let waker = waker_ref(&task_waker);
                let mut ctx = Context::from_waker(&waker);

                if task.as_mut().poll(&mut ctx).is_pending() {
                    self.tasks.borrow_mut().insert(task_id, task);
                }
            }
            
            if !self.tasks.borrow().is_empty() && self.ready_queue.borrow().is_empty() {
                
                let wakers_to_wake = self.get_poller().poll()?;
                
                for waker in wakers_to_wake {
                    waker.wake();
                }
            }
            
        }
        
        Ok(())
    }

    pub fn read<'a, F>(fd: F, buf: &'a mut [u8]) -> ReadFuture<'a, F>
    where
        F: AsRawFd,
    {
        ReadFuture {
            fd,
            buf,
            state: FutureState::Unsubmitted,
        }
    }

    /// Creates a future that will write to a file descriptor.
    pub fn write<'a, F>(fd: F, buf: &'a [u8]) -> WriteFuture<'a, F>
    where
        F: AsRawFd,
    {
        WriteFuture {
            fd,
            buf,
            state: FutureState::Unsubmitted,
        }
    }
    pub fn accept<'a>(listener: &'a TcpListener) -> AcceptFuture<'a> {
        AcceptFuture::new(listener)
    }

    pub fn recv_from<'a, F>(fd: F, buf: &'a mut [u8]) -> RecvFromFuture<'a, F>
    where
        F: AsRawFd,
    {
        RecvFromFuture {
            fd,
            buf,
            state: FutureState::Unsubmitted,
        }
    }

    pub fn send_to<'a, F>(fd: F, buf: &'a [u8], addr: SocketAddr) -> SendToFuture<'a, F>
    where
        F: AsRawFd,
    {
        SendToFuture {
            fd,
            buf,
            addr,
            state: FutureState::Unsubmitted,
        }
    }

    fn new_task_id() -> TaskId {
        use std::sync::atomic::{AtomicU64, Ordering};
        static ID: AtomicU64 = AtomicU64::new(0);
        let id = ID.fetch_add(1, Ordering::Relaxed);
        TaskId(id)
    }
}

pub enum FutureState {
    Unsubmitted,
    Pending(u64),
    Done,
}

pub struct ReadFuture<'a, F> {
    fd: F,
    buf: &'a mut [u8],
    state: FutureState,
}

impl<'a, F> Future for ReadFuture<'a, F>
where
    F: AsRawFd + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| {
            match this.state {
                FutureState::Unsubmitted => {
                    let mut poller = local_proactor.get_poller();

                    // Submit the read operation
                    let token =
                        poller.submit_read_entry(this.fd.as_raw_fd(), this.buf, cx.waker().clone());

                    // Update state to pending
                    this.state = FutureState::Pending(token);
                    Poll::Pending
                }
                FutureState::Pending(token) => {
                    // this.state = FutureState::Done;

                    let mut poller = local_proactor.get_poller();
                    if let Some(res) = poller.get_result(token) {
                        this.state = FutureState::Done;
                        Poll::Ready(Ok(res))
                    } else {
                        Poll::Pending
                    }
                }
                FutureState::Done => {
                    // This should not happen if polled after completion, but we handle it.
                    panic!("Polled a completed future");
                }
            }
        })
    }
}

pub struct WriteFuture<'a, F> {
    fd: F,
    buf: &'a [u8],
    state: FutureState,
}

impl<'a, F> Future for WriteFuture<'a, F>
where
    F: AsRawFd + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| {
            match this.state {
                FutureState::Unsubmitted => {
                    let mut poller = local_proactor.get_poller();

                    // Submit the write operation (assuming a similar method exists in Poller)
                    let token = poller.submit_write_entry(
                        this.fd.as_raw_fd(),
                        this.buf,
                        cx.waker().clone(),
                    );

                    this.state = FutureState::Pending(token);
                    Poll::Pending
                }
                FutureState::Pending(token) => {
                    // this.state = FutureState::Done;

                    let mut poller = local_proactor.get_poller();
                    if let Some(res) = poller.get_result(token) {
                        this.state = FutureState::Done;
                        Poll::Ready(Ok(res))
                    } else {
                        Poll::Pending
                    }
                }
                FutureState::Done => {
                    panic!("Polled a completed future");
                }
            }
        })
    }
}

pub struct RecvFromFuture<'a, F> {
    fd: F,
    buf: &'a mut [u8],
    state: FutureState,
}

impl<'a, F> Future for RecvFromFuture<'a, F>
where
    F: AsRawFd + Unpin,
{
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| {
            match this.state {
                FutureState::Unsubmitted => {
                    let mut poller = local_proactor.get_poller();

                    // Submit the recv_from operation
                    let token = poller.submit_recv_from_entry(
                        this.fd.as_raw_fd(),
                        this.buf,
                        cx.waker().clone(),
                    );

                    // Update state to pending
                    this.state = FutureState::Pending(token);
                    Poll::Pending
                }
                FutureState::Pending(token) => {
                    // this.state = FutureState::Done;

                    // let mut poller = local_proactor.get_poller();
                    // let (res, addr) = poller.get_addr_result(token).unwrap();

                    let mut poller = local_proactor.get_poller();
                    if let Some((res, addr)) = poller.get_addr_result(token) {
                        this.state = FutureState::Done;
                        Poll::Ready(Ok((res, addr)))
                    } else {
                        Poll::Pending
                    }
                    // Poll::Ready(Ok((res, addr)))
                }
                FutureState::Done => {
                    // This should not happen if polled after completion, but we handle it.
                    panic!("Polled a completed future");
                }
            }
        })
    }
}

pub struct SendToFuture<'a, F> {
    fd: F,
    buf: &'a [u8],
    addr: SocketAddr,
    state: FutureState,
}

impl<'a, F> Future for SendToFuture<'a, F>
where
    F: AsRawFd + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Proactor::with(|local_proactor| {
            match this.state {
                FutureState::Unsubmitted => {
                    let mut poller = local_proactor.get_poller();

                    // Submit the send_to operation
                    let token = poller.submit_send_to_entry(
                        this.fd.as_raw_fd(),
                        this.buf,
                        this.addr,
                        cx.waker().clone(),
                    );

                    this.state = FutureState::Pending(token);
                    Poll::Pending
                }
                FutureState::Pending(token) => {
                    // this.state = FutureState::Done;

                    let mut poller = local_proactor.get_poller();
                    if let Some(res) = poller.get_result(token) {
                        this.state = FutureState::Done;
                        Poll::Ready(Ok(res))
                    } else {
                        Poll::Pending
                    }
                    // Poll::Ready(Ok(res))
                }
                FutureState::Done => {
                    panic!("Polled a completed future");
                }
            }
        })
    }
}
