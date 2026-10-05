/* Reverse RTSP events. Framing/AEAD reuse a separate HAP context; the
   pairing path derives its keys from the full SRP or X25519 session secret.
   Requests are acknowledged on this socket, never on the control socket.
   Modelled on pinned ap2_mrp.c and pyatv's EventChannel. */
#define PROBE_EVENT_HEADER_MAX 8192
#define PROBE_EVENT_PLAIN_MAX 65536
struct probe_events {
    struct ap2cl_s *owner;
    struct ap2_hap_ctx *crypto;
    pthread_t thread;
    atomic_bool stop;
    bool failed;
    uint64_t requests, replies;
    uint8_t encrypted[8192], plain[PROBE_EVENT_PLAIN_MAX];
    size_t encrypted_len, plain_len;
};
static bool probe_event_fail(struct probe_events *e, const char *reason) {
    if (!e->failed) ap2_io_status_line("[PROBE] EVENTS_FAILED reason=%s host=%s", reason,e->owner->device.address);
    e->failed = true;
    atomic_store(&e->owner->media_healthy, false);
    return false;
}
static void probe_event_safe_text(char *text) {
    for (char *c=text; *c; ++c) if ((unsigned char)*c<32 || (unsigned char)*c>126) *c='.';
}
static bool probe_event_requests(struct probe_events *e) {
    for (;;) {
        size_t header_len=0;
        for (size_t i=0; i+3<e->plain_len; ++i) {
            if (!memcmp(e->plain+i,"\r\n\r\n",4)) { header_len=i+4; break; }
        }
        if (!header_len) {
            if (e->plain_len>=PROBE_EVENT_HEADER_MAX) return probe_event_fail(e,"header-too-large");
            return true;
        }
        if (header_len>=PROBE_EVENT_HEADER_MAX) return probe_event_fail(e,"header-too-large");
        char header[PROBE_EVENT_HEADER_MAX];
        memcpy(header,e->plain,header_len); header[header_len]=0;
        for (size_t i=0;i<header_len;++i) {
            unsigned char c=(unsigned char)header[i];
            if (c==0 || (c<32 && c!='\r' && c!='\n' && c!='\t') || c==127)
                return probe_event_fail(e,"invalid-header");
        }
        char *line=strstr(header,"\r\n");
        if (!line) return probe_event_fail(e,"invalid-request-line");
        *line=0;
        char method[16], path[256], version[16], extra;
        if (sscanf(header,"%15s %255s %15s %c",method,path,version,&extra)!=3 ||
            (strcmp(version,"RTSP/1.0") && strcmp(version,"HTTP/1.1") && strcmp(version,"HTTP/1.0")))
            return probe_event_fail(e,"invalid-request-line");
        size_t body_len=0; bool have_length=false, have_cseq=false;
        char cseq[32]="";
        line+=2;
        while (*line) {
            char *end=strstr(line,"\r\n");
            if (!end) return probe_event_fail(e,"invalid-header");
            if (end==line) break;
            *end=0;
            char *colon=strchr(line,':');
            if (!colon) return probe_event_fail(e,"invalid-header");
            *colon=0; char *value=colon+1;
            while (*value==' ' || *value=='\t') ++value;
            char *trim=end;
            while (trim>value && (trim[-1]==' ' || trim[-1]=='\t')) *--trim=0;
            if (!strcasecmp(line,"Content-Length")) {
                if (have_length || !*value) return probe_event_fail(e,"invalid-content-length");
                have_length=true;
                for (char *c=value;*c;++c) {
                    if (*c<'0' || *c>'9') return probe_event_fail(e,"invalid-content-length");
                    if (body_len>((size_t)PROBE_EVENT_PLAIN_MAX-(size_t)(*c-'0'))/10)
                        return probe_event_fail(e,"body-too-large");
                    body_len=body_len*10+(size_t)(*c-'0');
                }
            } else if (!strcasecmp(line,"CSeq")) {
                if (have_cseq || !*value || strlen(value)>=sizeof(cseq))
                    return probe_event_fail(e,"invalid-cseq");
                for (char *c=value;*c;++c) if (*c<'0' || *c>'9') return probe_event_fail(e,"invalid-cseq");
                strcpy(cseq,value); have_cseq=true;
            } else if (!strcasecmp(line,"Transfer-Encoding")) {
                return probe_event_fail(e,"unsupported-transfer-encoding");
            }
            line=end+2;
        }
        if (body_len>PROBE_EVENT_PLAIN_MAX-header_len) return probe_event_fail(e,"body-too-large");
        if (header_len+body_len>e->plain_len) return true;
        char type[96]="-", value[96]="-";
        bool command=!strcmp(method,"POST") && !strcmp(path,"/command");
        if (command && body_len>=8 && !memcmp(e->plain+header_len,"bplist00",8)) {
            ap2_bplist_get_root_string(e->plain+header_len,body_len,"type",type,sizeof(type));
            ap2_bplist_get_root_string(e->plain+header_len,body_len,"value",value,sizeof(value));
        }
        probe_event_safe_text(type); probe_event_safe_text(value);
        if (!type[0]) strcpy(type,"-");
        if (!value[0]) strcpy(value,"-");
        probe_event_safe_text(method); probe_event_safe_text(path);
        ++e->requests;
        char response[256];
        int size=snprintf(response,sizeof(response),
            "%s 200 OK\r\nContent-Length: 0\r\nAudio-Latency: 0\r\n%s%s%s\r\n",
            version,have_cseq?"CSeq: ":"",cseq,have_cseq?"\r\n":"");
        uint8_t *wire=NULL;
        int wire_len=ap2_hap_encrypt(e->crypto,(const uint8_t *)response,size,&wire);
        bool ok=wire_len>0 && wire && ap2_io_write_all_deadline(e->owner->events_sock,
            wire,(size_t)wire_len,ap2_io_monotonic_ms()+1000);
        free(wire);
        if (!ok) return probe_event_fail(e,"response-write");
        ++e->replies;
        // ACK confirms receipt; playback commands are logged, not executed.
        ap2_io_status_line("[PROBE] EVENT method=%s path=%s cseq=%s bytes=%lu type=%s value=%s reply=200 host=%s",
            method,path,have_cseq?cseq:"-",(unsigned long)body_len,type,value,e->owner->device.address);
        size_t used=header_len+body_len;
        memmove(e->plain,e->plain+used,e->plain_len-used); e->plain_len-=used;
    }
}
static bool probe_event_frames(struct probe_events *e) {
    while (e->encrypted_len>=2) {
        size_t size=(size_t)e->encrypted[0]+((size_t)e->encrypted[1]<<8);
        if (!size || size>1024) return probe_event_fail(e,"invalid-frame-length");
        size_t frame_len=size+18;
        if (e->encrypted_len<frame_len) break;
        uint8_t *plain=NULL;
        int count=ap2_hap_decrypt(e->crypto,e->encrypted,(int)frame_len,&plain);
        if (count!=(int)size || !plain) { free(plain); return probe_event_fail(e,"authentication"); }
        if (e->plain_len+size>sizeof(e->plain)) { free(plain); return probe_event_fail(e,"plain-buffer-full"); }
        memcpy(e->plain+e->plain_len,plain,size); e->plain_len+=size; free(plain);
        memmove(e->encrypted,e->encrypted+frame_len,e->encrypted_len-frame_len);
        e->encrypted_len-=frame_len;
        if (!probe_event_requests(e)) return false;
    }
    return true;
}
static void *probe_event_worker(void *arg) {
    struct probe_events *e=arg;
    while (!atomic_load(&e->stop)) {
        struct pollfd fd={.fd=(SOCKET)e->owner->events_sock,.events=POLLIN};
        int ready=poll(&fd,1,50);
        if (atomic_load(&e->stop)) break;
        if (ready<0) { probe_event_fail(e,"socket-poll"); break; }
        if (!ready) continue;
        if (!(fd.revents&POLLIN)) { probe_event_fail(e,"socket-closed"); break; }
        size_t space=sizeof(e->encrypted)-e->encrypted_len;
        if (!space) { probe_event_fail(e,"encrypted-buffer-full"); break; }
        int n=recv(e->owner->events_sock,e->encrypted+e->encrypted_len,space,0);
        if (n<0 && (errno==EAGAIN || errno==EWOULDBLOCK || errno==EINTR)) continue;
        if (n<=0) { probe_event_fail(e,"socket-closed"); break; }
        e->encrypted_len+=(size_t)n;
        if (!probe_event_frames(e)) break;
    }
    return NULL;
}
static bool probe_events_start(struct ap2cl_s *p) {
    if (p->events_sock<0) return true;
    struct probe_events *e=calloc(1,sizeof(*e));
    if (!e) { ap2_io_status_line("[PROBE] EVENTS_FAILED reason=allocation"); return false; }
    e->owner=p; e->crypto=probe_hap_events_create(p->hap); atomic_init(&e->stop,false);
    u_long mode=1;
    if (!e->crypto || ioctlsocket((SOCKET)p->events_sock,FIONBIO,&mode) ||
        pthread_create(&e->thread,NULL,probe_event_worker,e)) {
        ap2_hap_destroy(e->crypto); free(e);
        ap2_io_status_line("[PROBE] EVENTS_FAILED reason=start"); return false;
    }
    p->events=e;
    ap2_io_status_line("[PROBE] EVENTS_READY encrypted=true");
    return true;
}
static bool probe_events_stop(struct ap2cl_s *p) {
    struct probe_events *e=p->events;
    if (!e) return true;
    atomic_store(&e->stop,true); pthread_join(e->thread,NULL);
    bool ok=!e->failed;
    ap2_io_status_line("[PROBE] EVENTS_STATS requests=%" PRIu64 " replies=%" PRIu64 " failed=%s host=%s",
        e->requests,e->replies,e->failed?"true":"false",p->device.address);
    ap2_hap_destroy(e->crypto); SecureZeroMemory(e,sizeof(*e)); free(e); p->events=NULL;
    return ok;
}
