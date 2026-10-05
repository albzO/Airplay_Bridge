#include "windows_port.h"
#include "probe.h"
#include <conio.h>

static void help(void) {
    puts("cliairplay-probe (control session, optional finite test tone)\n"
         "--host <IPv4> --port <discovered port> [--name <name>] [--txt <TXT>]\n"
         "[--password-auto [--password-stdin] | --password-prompt | --password-stdin | --password <value>]\n"
         "[--timing ptp|ntp] [--hold-seconds 0..30] [--bind-ip <local IPv4>] [--tone]\n"
         "[--pcm-file <raw 44100Hz S16LE stereo file, <=60 seconds>]\n"
         "[--pcm-stdin (binary 44100Hz S16LE stereo pipe, EOF stops stream)]\n"
         "[--latency-ms 250..2000] [--buffer-ms 64..512]\n"
         "[--peer-host <IPv4> --peer-port <port> --peer-name <name> --peer-txt <TXT>\n"
         " --peer-identity <16 hex> --peer-active-remote <uint32> --peer-volume-control-port <port>]\n"
         "[--volume-control-port <local callback bridge>] [--active-remote <uint32>]\n"
         "[--auth <stored credentials>] [--identity <16 hex chars>] [--debug]\n"
         "--self-check checks dependencies without contacting a receiver.");
}
static char *utf8(const wchar_t *value) {
    int size = WideCharToMultiByte(CP_UTF8, 0, value, -1, NULL, 0, NULL, NULL);
    char *result = malloc((size_t)size);
    if (result) WideCharToMultiByte(CP_UTF8, 0, value, -1, result, size, NULL, NULL);
    return result;
}
typedef struct { char *buffer; int size; bool input; const char *pipe; } password_request;
static bool password_pipe_io(HANDLE pipe,void *data,DWORD length,bool writing) {
    DWORD done=0;
    while (done<length) {
        DWORD count=0;
        BOOL ok=writing?WriteFile(pipe,(char *)data+done,length-done,&count,NULL):ReadFile(pipe,(char *)data+done,length-done,&count,NULL);
        if (!ok || !count) return false;
        done+=count;
    }
    return true;
}
static const char *request_password(const char *host, const char *name, void *arg) {
    password_request *request=arg;
    if (*request->buffer) {
        fprintf(stderr,"[PROBE] PASSWORD_REUSED host=%s\n",host);
        return request->buffer;
    }
    fprintf(stderr,"[PROBE] PASSWORD_NEEDED host=%s phase=pairing\n",host);
    if (request->pipe) {
        HANDLE pipe=CreateFileA(request->pipe,GENERIC_READ|GENERIC_WRITE,0,NULL,OPEN_EXISTING,0,NULL);
        if (pipe==INVALID_HANDLE_VALUE) return NULL;
        uint32_t host_len=(uint32_t)strlen(host),password_len=0;
        bool ok=password_pipe_io(pipe,&host_len,sizeof(host_len),true) &&
            password_pipe_io(pipe,(void *)host,host_len,true) &&
            password_pipe_io(pipe,&password_len,sizeof(password_len),false) &&
            password_len>0 && password_len<(uint32_t)request->size &&
            password_pipe_io(pipe,request->buffer,password_len,false);
        CloseHandle(pipe);
        if (!ok) { SecureZeroMemory(request->buffer,request->size);return NULL; }
        request->buffer[password_len]=0;
    } else if (request->input) {
        if (!fgets(request->buffer,request->size,stdin)) return NULL;
        request->buffer[strcspn(request->buffer,"\r\n")]=0;
    } else {
        wchar_t hidden[256]={0}; size_t length=0;
        fprintf(stderr,"AirPlay password required for %s (%s), hidden: ",name?name:"HomePod",host);
        for (;;) {
            wint_t c=_getwch();
            if (c==L'\r') break;
            if (c==3) { SecureZeroMemory(hidden,sizeof(hidden)); return NULL; }
            if (c==L'\b') { if (length) length--; continue; }
            if (c==0 || c==0xe0) { _getwch(); continue; }
            if (length+1<256) hidden[length++]=(wchar_t)c;
        }
        hidden[length]=0;
        WideCharToMultiByte(CP_UTF8,0,hidden,-1,request->buffer,request->size,NULL,NULL);
        SecureZeroMemory(hidden,sizeof(hidden));
        fputc('\n',stderr);
    }
    return *request->buffer?request->buffer:NULL;
}
int wmain(int argc, wchar_t **wide) {
    SetConsoleOutputCP(CP_UTF8);
    char **argv = calloc((size_t)argc, sizeof(*argv));
    for (int i = 0; i < argc; i++) argv[i] = utf8(wide[i]);
    probe_options options = {.name = "HomePod Probe", .identity = "A1B2C3D4E5F60718", .active_remote = "1", .hold_seconds = 4, .use_ptp = true, .lead_ms = 2000, .buffer_ms = 128};
    bool prompt = false, input = false, automatic = false;
    char password[1024] = {0};
    const char *password_pipe=NULL;
    bool password_pipe_first=false;
    int result = 2;
    for (int i = 1; i < argc; i++) {
        char *arg = argv[i];
        if (strcmp(arg, "--help") == 0) { help(); result = 0; goto finish; }
        if (strcmp(arg, "--self-check") == 0) {
            if (!probe_nonce_selfcheck()) { fputs("Audio nonce check failed\n", stderr); goto finish; }
            puts("64-bit audio nonce: 65538 counter values and exhaustion check passed");
            puts("Windows x64 session probe: ready; no receiver contacted");
            result = 0; goto finish;
        }
        if (strcmp(arg, "--password-prompt") == 0) { prompt = true; continue; }
        if (strcmp(arg, "--password-auto") == 0) { automatic = true; continue; }
        if (strcmp(arg,"--password-pipe-first")==0) { password_pipe_first=true;continue; }
        if (strcmp(arg,"--peer-password-first")==0) { options.peer_password_first=true;continue; }
        if (strcmp(arg, "--password-stdin") == 0) { input = true; continue; }
        if (strcmp(arg, "--debug") == 0) { probe_debug = 1; continue; }
        if (strcmp(arg, "--tone") == 0) { options.play_tone = true; continue; }
        if (strcmp(arg, "--pcm-stdin") == 0) { options.pcm_stdin = true; continue; }
        if (i + 1 >= argc) { fprintf(stderr, "Missing value for %s\n", arg); goto finish; }
        char *value = argv[++i];
        if (strcmp(arg, "--host") == 0) options.host = value;
        else if (strcmp(arg, "--port") == 0) options.port = atoi(value);
        else if (strcmp(arg, "--name") == 0) options.name = value;
        else if (strcmp(arg, "--txt") == 0) options.txt = value;
        else if (strcmp(arg, "--peer-host") == 0) options.peer_host = value;
        else if (strcmp(arg, "--peer-name") == 0) options.peer_name = value;
        else if (strcmp(arg, "--peer-txt") == 0) options.peer_txt = value;
        else if (strcmp(arg, "--peer-port") == 0) options.peer_port = atoi(value);
        else if (strcmp(arg, "--peer-identity") == 0) options.peer_identity = value;
        else if (strcmp(arg, "--peer-active-remote") == 0) options.peer_active_remote = value;
        else if (strcmp(arg, "--peer-volume-control-port") == 0) options.peer_volume_control_port = atoi(value);
        else if (strcmp(arg, "--password") == 0) options.password = value;
        else if (strcmp(arg,"--password-pipe")==0) { password_pipe=value;automatic=true; }
        else if (strcmp(arg, "--auth") == 0) options.credentials = value;
        else if (strcmp(arg, "--pcm-file") == 0) options.pcm_file = value;
        else if (strcmp(arg, "--identity") == 0) options.identity = value;
        else if (strcmp(arg, "--bind-ip") == 0) options.bind_ip = value;
        else if (strcmp(arg, "--hold-seconds") == 0) options.hold_seconds = atoi(value);
        else if (strcmp(arg, "--latency-ms") == 0) options.lead_ms = atoi(value);
        else if (strcmp(arg, "--buffer-ms") == 0) options.buffer_ms = atoi(value);
        else if (strcmp(arg, "--volume-control-port") == 0) options.volume_control_port = atoi(value);
        else if (strcmp(arg, "--active-remote") == 0) options.active_remote = value;
        else if (strcmp(arg, "--timing") == 0 && (strcmp(value, "ptp") == 0 || strcmp(value, "ntp") == 0)) options.use_ptp = strcmp(value, "ptp") == 0;
        else { fprintf(stderr, "Unknown option/value: %s\n", arg); goto finish; }
    }
    struct in_addr parsed;
    if (options.peer_host && (!options.pcm_stdin || !options.use_ptp ||
        inet_pton(AF_INET,options.peer_host,&parsed)!=1 || options.peer_port<1 || options.peer_port>65535 ||
        !options.peer_identity || strlen(options.peer_identity)!=16 ||
        strspn(options.peer_identity,"0123456789abcdefABCDEF")!=16 ||
        !options.peer_active_remote || !*options.peer_active_remote || strlen(options.peer_active_remote)>10 ||
        strspn(options.peer_active_remote,"0123456789")!=strlen(options.peer_active_remote) ||
        strtoull(options.peer_active_remote,NULL,10)>UINT32_MAX ||
        options.peer_volume_control_port<0 || options.peer_volume_control_port>65535)) {
        fputs("Invalid stereo peer options (requires PTP and PCM stdin)\n",stderr); goto finish;
    }
    if (!options.host || inet_pton(AF_INET, options.host, &parsed) != 1 || options.port < 1 || options.port > 65535 ||
        options.hold_seconds < 0 || options.hold_seconds > 30 || strlen(options.identity) != 16 ||
        options.lead_ms < 250 || options.lead_ms > 2000 || options.buffer_ms < 64 || options.buffer_ms > 512 ||
        options.volume_control_port < 0 || options.volume_control_port > 65535 ||
        (options.bind_ip && inet_pton(AF_INET, options.bind_ip, &parsed) != 1) ||
        (prompt + input + (options.password != NULL) > 1) ||
        (automatic && (prompt || options.password || options.credentials)) ||
        (password_pipe && (input || strncmp(password_pipe,"\\\\.\\pipe\\airplay-bridge-",24)!=0 || strlen(password_pipe)>240)) ||
        (password_pipe_first && !password_pipe) ||
        (options.peer_password_first && (!password_pipe || !options.peer_host)) ||
        (options.play_tone + (options.pcm_file != NULL) + options.pcm_stdin > 1) ||
        (options.pcm_stdin && input)) {
        help(); goto finish;
    }
    if (!*options.active_remote || strlen(options.active_remote)>10 ||
        strspn(options.active_remote,"0123456789")!=strlen(options.active_remote) ||
        strtoull(options.active_remote,NULL,10)>UINT32_MAX) {
        fputs("Invalid Active-Remote value\n",stderr);goto finish;
    }
    for (const char *c = options.identity; *c; c++)
        if (!((*c >= '0' && *c <= '9') || (*c >= 'A' && *c <= 'F') || (*c >= 'a' && *c <= 'f'))) {
            fputs("Identity must contain 16 hex characters\n", stderr); goto finish;
        }
    if (options.pcm_file) {
        uint64_t frames;
        FILE *source = probe_pcm_open(options.pcm_file, &frames);
        if (!source) { fprintf(stderr, "Invalid PCM file (raw 44100Hz S16LE stereo, <=60s): %s\n", strerror(errno)); goto finish; }
        fclose(source);
    }
    password_request request={password,sizeof(password),input,password_pipe};
    if (automatic) {
        options.request_password=request_password;
        options.password_arg=&request;
        if (password_pipe_first) {
            if (!request_password(options.host,options.name,&request)) {result=11;goto finish;}
            options.password=password;
        }
    } else if (prompt) {
        wchar_t hidden[256] = {0}; size_t length = 0;
        fputs("AirPlay password (hidden): ", stderr);
        for (;;) {
            wint_t c = _getwch();
            if (c == L'\r') break;
            if (c == 3) { SecureZeroMemory(hidden, sizeof(hidden)); goto finish; }
            if (c == L'\b') { if (length) length--; continue; }
            if (c == 0 || c == 0xe0) { _getwch(); continue; }
            if (length + 1 < 256) hidden[length++] = (wchar_t)c;
        }
        hidden[length] = 0;
        WideCharToMultiByte(CP_UTF8, 0, hidden, -1, password, sizeof(password), NULL, NULL);
        SecureZeroMemory(hidden, sizeof(hidden));
        fputc('\n', stderr); options.password = password;
    } else if (input) {
        if (!fgets(password, sizeof(password), stdin)) { fputs("No password received on stdin\n", stderr); goto finish; }
        password[strcspn(password, "\r\n")] = 0; options.password = password;
    }
    if (options.pcm_stdin) {
        if (GetFileType(GetStdHandle(STD_INPUT_HANDLE)) != FILE_TYPE_PIPE) {
            fputs("PCM stdin requires a binary pipe\n", stderr); goto finish;
        }
        /* Parent handles Ctrl+C, closes the PCM pipe, and lets us TEARDOWN. */
        SetConsoleCtrlHandler(NULL, TRUE);
    }
    WSADATA data;
    if (WSAStartup(MAKEWORD(2, 2), &data) != 0) { fputs("WSAStartup failed\n", stderr); goto finish; }
    result = probe_run(&options);
    WSACleanup();
finish:
    SecureZeroMemory(password, sizeof(password));
    for (int i = 0; i < argc; i++) { if (argv[i]) { SecureZeroMemory(argv[i], strlen(argv[i])); free(argv[i]); } }
    free(argv);
    return result;
}
