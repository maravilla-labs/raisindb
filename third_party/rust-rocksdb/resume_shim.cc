// rocksdb::DB::Resume() for the Rust binding. The C API has no equivalent.
//
// `rocksdb_t` is declared opaque in rocksdb/c.h and defined in db/c.cc as
// `struct rocksdb_t { DB* rep; };`. It is redefined identically here so the
// handle rust-rocksdb holds can be followed to the DB.

#include <cstdlib>
#include <cstring>
#include <string>

#include "rocksdb/db.h"

struct rocksdb_t {
  rocksdb::DB* rep;
};

extern "C" {

// Returns NULL when the database is writable afterwards (including when it
// was never stopped: Resume() on a healthy DB is a no-op), otherwise a
// malloc'd error string the caller frees with free().
char* raisin_rocksdb_resume(rocksdb_t* db) {
  rocksdb::Status s = db->rep->Resume();
  if (s.ok()) {
    return nullptr;
  }
  return strdup(s.ToString().c_str());
}

}  // extern "C"
