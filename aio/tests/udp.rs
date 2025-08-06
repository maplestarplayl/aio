use uring_rt::net::UdpSocket;

#[aio::test]
pub async fn test_udp_echo() {
    let socket = UdpSocket::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let local_addr = socket.local_addr().unwrap();

    let client_socket = UdpSocket::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let client_addr = client_socket.local_addr().unwrap();

    let send_buf = b"hello";
    client_socket.send_to(send_buf, local_addr).await.unwrap();

    let mut recv_buf = [0u8; 1024];
    let (len, addr) = socket.recv_from(&mut recv_buf).await.unwrap();

    assert_eq!(addr, client_addr);
    assert_eq!(&recv_buf[..len], send_buf);

    socket.send_to(&recv_buf[..len], addr).await.unwrap();

    let (len, addr) = client_socket.recv_from(&mut recv_buf).await.unwrap();

    assert_eq!(addr, local_addr);
    assert_eq!(&recv_buf[..len], send_buf);
}
