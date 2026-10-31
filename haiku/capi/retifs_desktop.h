/*
 * retifs-desktop's shared core, as C (haiku/capi): JSON in and out.
 * Every returned string is the caller's, freed with rd_free(). Actions
 * answer "" when they went well, else what went wrong. Calls block.
 */
#ifndef RETIFS_DESKTOP_H
#define RETIFS_DESKTOP_H

#ifdef __cplusplus
extern "C" {
#endif

char* rd_status(void);      /* {"up", "store", "node", "chunks", "usage", "error", "libraries"} */
char* rd_heard(void);       /* [{"name", "about", "address", "node", "hops", ...}] or {"error"} */
char* rd_reticulum(void);   /* {"running", "transport", "uptime", "error", "connections"} */
char* rd_settings(void);    /* [{"section", "key", "what", "switch", "value"}] */
char* rd_services(void);    /* [{"name", "what", "state"}] */

char* rd_join(const char* address);
char* rd_leave(const char* ns);
char* rd_add_tcp(const char* name, const char* host, int port);
char* rd_set_interface(const char* name, int enabled);
char* rd_remove_interface(const char* name);
char* rd_save_settings(const char* json);   /* {"section.key": "value" | null} */
char* rd_service(const char* name, const char* action);   /* start, stop, restart */

void rd_free(char* s);

#ifdef __cplusplus
}
#endif

#endif
