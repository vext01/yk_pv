#define _GNU_SOURCE

#include <inttypes.h>
#include <jit-reader.h>
#include <stdio.h>
#include <stdlib.h>

GDB_DECLARE_GPL_COMPATIBLE_READER

// gdb is single-threaded, so this doesn't need to be synchronised.
size_t trace_number = 0;
#define TRACE_NAME_PREFIX "__yk_compiled_trace"

#define MAX_FILENAME 256

// Write the debug info into a buffer in our custom format:
//
// code_vaddr: usize
// code_size: usize
// num_lineinfo_pairs: usize
// lineinfo_key[0]: usize
// lineinfo_val[0]: usize
// ...
// lineinfo_key[num_lineinfo_pairs - 1]: usize
// lineinfo_val[num_lineinfo_pairs - 1]: usize
// src_filename: char[src_filename_len] (null terminated)

enum gdb_status read_debug_info_cb(struct gdb_reader_funcs *self,
                                   struct gdb_symbol_callbacks *cb,
                                   void *memory, long memory_sz) {
  // Let's address the memory in uintptr_t-sized chunks.
  uintptr_t *payload = (uintptr_t *)memory;

  uintptr_t code_vaddr = *payload++;
  size_t code_size = *payload++;
  size_t num_lineinfo_pairs = *payload++;
  // FIXME: violates pointer aliasing rules? Copy?
  struct gdb_line_mapping *lineinfo_pairs = (struct gdb_line_mapping *) payload;
  for (size_t i = 0; i < num_lineinfo_pairs; i++) {
    payload += 2; // FIXME makes assumptions about x86_64 FIXME
  }
#include <string.h>
  char *src_filename = strdup((char *) payload);

  struct gdb_object *obj = cb->object_open(cb);

  // FIXME: free it later.
  char *trace_name = NULL;
  if (asprintf(&trace_name, TRACE_NAME_PREFIX "%zu", trace_number++) == -1) {
    fprintf(stderr, "asprintf failed\n");
    exit(EXIT_FAILURE);
  }

  struct gdb_symtab *symtab = cb->symtab_open(cb, obj, src_filename);
  struct gdb_block *code_block =
      cb->block_open(cb, symtab, NULL, (GDB_CORE_ADDR)code_vaddr,
                     (GDB_CORE_ADDR)(code_vaddr + code_size), trace_name);

  cb->line_mapping_add(cb, symtab, num_lineinfo_pairs, lineinfo_pairs);

  cb->symtab_close(cb, symtab);
  cb->object_close(cb, obj);
  return GDB_SUCCESS;
}

void destory_reader_cb(struct gdb_reader_funcs *self) {
}

enum gdb_status unwind_frame_cb(struct gdb_reader_funcs *self,
                                struct gdb_unwind_callbacks *cb) {
  return GDB_FAIL;
}

struct gdb_frame_id get_frame_id_cb(struct gdb_reader_funcs *self,
                                    struct gdb_unwind_callbacks *c) {
  struct gdb_frame_id ret = {0, 0};
  return ret;
}

struct gdb_reader_funcs reader_funcs = {
    .reader_version = GDB_READER_INTERFACE_VERSION,
    .priv_data = NULL,
    .read = read_debug_info_cb,
    .unwind = unwind_frame_cb,
    .get_frame_id = get_frame_id_cb,
    .destroy = destory_reader_cb,
};

struct gdb_reader_funcs *gdb_init_reader(void) {
  return &reader_funcs;
}
