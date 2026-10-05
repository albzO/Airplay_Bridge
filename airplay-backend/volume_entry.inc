/* Device callbacks arrive over a separate localhost control connection.
   PCM gain stays unity. Reports update state without echoing SET_PARAMETER. */
static bool probe_volume_value(const char *text, double *value) {
    char *end; errno=0; *value=strtod(text,&end);
    while (*end==' ' || *end=='\r' || *end=='\n') end++;
    return end!=text && !*end && !errno && isfinite(*value);
}
static bool probe_volume_query(struct ap2cl_s *p) {
    static const char query[]="volume\r\n";
    uint8_t *body=NULL; int length=0;
    int status=ap2_rtsp_send(p,"GET_PARAMETER",p->session_url,(const uint8_t *)query,
                           sizeof(query)-1,"text/parameters",&body,&length);
    double db=0; bool found=false;
    if (status==200 && body && length>0 && length<256) {
        char text[256]; memcpy(text,body,(size_t)length);text[length]=0;
        char *value=strstr(text,"volume:");
        if (value && probe_volume_value(value+7,&db) && db>=-144 && db<=0) {
            p->volume_db=db;p->volume_known=true;found=true;
            ap2_io_status_line("[PROBE] VOLUME_CURRENT source=query db=%.3f host=%s",db,p->device.address);
        }
    }
    free(body);
    if (!found) ap2_io_status_line("[PROBE] VOLUME_QUERY_UNAVAILABLE http=%d (waiting for device callback)",status);
    return found;
}
static bool probe_volume_start(struct ap2cl_s *p,const probe_options *options) {
    if (!options->volume_control_port) return true;
    p->volume_sock=socket(AF_INET,SOCK_STREAM,0);
    if (p->volume_sock<0) return false;
    struct sockaddr_in address={.sin_family=AF_INET,.sin_port=htons(options->volume_control_port)};
    address.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
    if (connect(p->volume_sock,(struct sockaddr *)&address,sizeof(address)) ||
        !ap2_set_nonblocking(p->volume_sock,"volume control")) return false;
    ap2_io_status_line("[PROBE] VOLUME_CONTROL_READY port=%d",options->volume_control_port);
    /* Reading is optional; never overwrite a device with a guessed startup level. */
    probe_volume_query(p);
    return !atomic_load(&p->rtsp_dead);
}
static void probe_volume_command(struct ap2cl_s *p,char *command) {
    char *value=strchr(command,' '); double amount;
    if (!value) return;
    *value++=0;
    if (!probe_volume_value(value,&amount)) {
        ap2_io_status_line("[PROBE] VOLUME_REJECT reason=invalid-number");return;
    }
    if (!strcmp(command,"REPORT")) {
        if (amount < -144 || amount>0) return;
        p->volume_db=amount;p->volume_known=true;
        ap2_io_status_line("[PROBE] VOLUME_CURRENT source=device db=%.3f host=%s",amount,p->device.address);
        return;
    }
    double percent;
    if (!strcmp(command,"SET") && amount>=0 && amount<=100) percent=amount;
    else if (!strcmp(command,"STEP") && fabs(amount)<=10) {
        if (!p->volume_known && !probe_volume_query(p)) {
            ap2_io_status_line("[PROBE] VOLUME_REJECT reason=current-volume-unknown");return;
        }
        percent=p->volume_db<=-144 ? 0 : fmax(0,fmin(100,(p->volume_db+30)*100/30));
        percent=fmax(0,fmin(100,percent+amount));
    } else {ap2_io_status_line("[PROBE] VOLUME_REJECT reason=invalid-command");return;}
    double db=percent<=0 ? -144 : percent*0.3-30;
    if (p->volume_known && fabs(db-p->volume_db)<0.05) return;
    char body[64]; int length=snprintf(body,sizeof(body),"volume: %.6f\r\n",db);
    uint8_t *reply=NULL;int reply_len=0;
    int status=ap2_rtsp_send(p,"SET_PARAMETER",p->session_url,(const uint8_t *)body,
                           length,"text/parameters",&reply,&reply_len);
    free(reply);
    ap2_io_status_line("[PROBE] VOLUME_APPLIED source=device-request percent=%.2f db=%.3f http=%d host=%s",percent,db,status,p->device.address);
    if (status==200) {p->volume_db=db;p->volume_known=true;}
}
static void probe_volume_tick(struct ap2cl_s *p) {
    if (p->volume_sock<0) return;
    struct pollfd fd={.fd=p->volume_sock,.events=POLLIN};
    if (poll(&fd,1,0)<=0 || !(fd.revents&(POLLIN|POLLHUP|POLLERR))) return;
    char block[128];int count=recv(p->volume_sock,block,sizeof(block),0);
    if (count<=0) {
        if (count<0 && errno==EWOULDBLOCK) return;
        ap2_io_status_line("[PROBE] VOLUME_CONTROL_CLOSED");close(p->volume_sock);p->volume_sock=-1;return;
    }
    for (int i=0;i<count;i++) {
        if (block[i]=='\n') {
            p->volume_line[p->volume_line_len]=0;
            probe_volume_command(p,p->volume_line);p->volume_line_len=0;
        } else if (p->volume_line_len<(int)sizeof(p->volume_line)-1) {
            p->volume_line[p->volume_line_len++]=block[i];
        } else {
            ap2_io_status_line("[PROBE] VOLUME_REJECT reason=oversized-command");
            close(p->volume_sock);p->volume_sock=-1;p->volume_line_len=0;return;
        }
    }
}
