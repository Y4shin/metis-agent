/*
 * A plugin built for another ABI major.
 *
 * It exports `metis_plugin_entry_v2` and nothing else. The major is part of the
 * entry symbol's name, so a host speaking major 1 cannot resolve a symbol to
 * call: it finds no table at all, which is the symbol half of the major check
 * and the reason a plugin from another major is refused before any table byte
 * is interpreted.
 *
 * The harness loads this library to prove the refusal is a typed error naming
 * both majors rather than the loader's own error about the byte string it
 * looked for. Nothing calls the function below, and no table is reachable
 * through it.
 */
#include "metis_plugin_abi.h"

const MetisPluginTableV1 *metis_plugin_entry_v2(void) {
    return NULL;
}
