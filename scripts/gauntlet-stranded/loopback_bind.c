// loopback_bind: DYLD interposer for the GS-N01 harness (Run 0b, INC-I-299). Test harness only.
//  1. bind() to INADDR_ANY / in6addr_any becomes 127.0.0.1 / ::1: the node listens on loopback only,
//     so no fleet node can dial it on a LAN or VM-bridge address.
//  2. LB_PORT_MAP="REAL:PUBLIC" (set by tn.spawn for P and Q from --p2p-port and --external-address):
//     getsockname() of the LISTENING socket on REAL reports PUBLIC, so the node announces only its
//     forwarder port and a fleet node never learns 127.0.0.1:REAL (a loopback dial past the forwarder
//     made the 17:31Z build NOT_MADE). A bind to PUBLIC (port reuse of the announced address) is sent
//     to REAL, so the node never collides with its own forwarder.
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static int lb_ports(unsigned *real, unsigned *pub) {
    const char *m = getenv("LB_PORT_MAP");
    return m && sscanf(m, "%u:%u", real, pub) == 2;
}

int lb_bind(int fd, const struct sockaddr *a, socklen_t l) {
    unsigned real, pub;
    int map = lb_ports(&real, &pub);
    if (a && a->sa_family == AF_INET && l >= sizeof(struct sockaddr_in)) {
        struct sockaddr_in s; memcpy(&s, a, sizeof s);
        if (s.sin_addr.s_addr == htonl(INADDR_ANY)) s.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        if (map && ntohs(s.sin_port) == pub) s.sin_port = htons((unsigned short)real);
        return bind(fd, (struct sockaddr *)&s, sizeof s);
    }
    if (a && a->sa_family == AF_INET6 && l >= sizeof(struct sockaddr_in6)) {
        struct sockaddr_in6 s; memcpy(&s, a, sizeof s);
        if (memcmp(&s.sin6_addr, &in6addr_any, sizeof s.sin6_addr) == 0) s.sin6_addr = in6addr_loopback;
        if (map && ntohs(s.sin6_port) == pub) s.sin6_port = htons((unsigned short)real);
        return bind(fd, (struct sockaddr *)&s, sizeof s);
    }
    return bind(fd, a, l);
}

int lb_getsockname(int fd, struct sockaddr *a, socklen_t *l) {
    int r = getsockname(fd, a, l);
    unsigned real, pub;
    if (r != 0 || !a || !lb_ports(&real, &pub)) return r;
    // only sockets bound to REAL match: the listener and port-reuse dial sockets of this node
    if (a->sa_family == AF_INET && ntohs(((struct sockaddr_in *)a)->sin_port) == real)
        ((struct sockaddr_in *)a)->sin_port = htons((unsigned short)pub);
    else if (a->sa_family == AF_INET6 && ntohs(((struct sockaddr_in6 *)a)->sin6_port) == real)
        ((struct sockaddr_in6 *)a)->sin6_port = htons((unsigned short)pub);
    return r;
}

__attribute__((used)) static struct { const void *n; const void *o; } interposers[]
    __attribute__((section("__DATA,__interpose"))) = {
        { (const void *)lb_bind, (const void *)bind },
        { (const void *)lb_getsockname, (const void *)getsockname } };
