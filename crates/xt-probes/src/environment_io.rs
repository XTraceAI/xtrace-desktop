//! Private read-only directory handles for the Environment probe.
//! All descriptor ownership and C calls live here. Paths after `/` are single
//! components, opened relative to a pinned parent, never resolved again.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native {
    use std::{
        ffi::{CStr, CString, OsStr},
        fs::{File, Metadata as FileMetadata},
        io,
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, OpenOptionsExt},
            },
        },
        path::{Component, Path},
        ptr::NonNull,
    };

    #[cfg(target_os = "macos")]
    const SEARCH_ONLY: libc::c_int = libc::O_SEARCH;
    #[cfg(target_os = "linux")]
    const SEARCH_ONLY: libc::c_int = libc::O_PATH;

    #[derive(Clone, Copy)]
    pub(crate) struct Metadata {
        dev: u64,
        ino: u64,
        mode: libc::mode_t,
        len: u64,
    }

    impl Metadata {
        pub(crate) fn is_dir(self) -> bool {
            self.mode & libc::S_IFMT == libc::S_IFDIR
        }
        pub(crate) fn is_file(self) -> bool {
            self.mode & libc::S_IFMT == libc::S_IFREG
        }
        pub(crate) fn is_symlink(self) -> bool {
            self.mode & libc::S_IFMT == libc::S_IFLNK
        }
        pub(crate) fn len(self) -> u64 {
            self.len
        }
        pub(crate) fn same_object(self, other: Self) -> bool {
            self.dev == other.dev
                && self.ino == other.ino
                && self.mode & libc::S_IFMT == other.mode & libc::S_IFMT
        }
        // mode_t is narrower on Darwin than Linux.
        #[allow(clippy::unnecessary_cast)]
        fn from_file(value: &FileMetadata) -> Self {
            Self {
                dev: value.dev(),
                ino: value.ino(),
                mode: value.mode() as libc::mode_t,
                len: value.len(),
            }
        }
    }

    pub(crate) struct DirHandle(File);

    impl DirHandle {
        pub(crate) fn clone_handle(&self) -> io::Result<Self> {
            self.0.try_clone().map(Self)
        }
        pub(crate) fn open_root(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let mut directory = Self(
                File::options()
                    .read(true)
                    .custom_flags(
                        SEARCH_ONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                    .open("/")?,
            );
            for part in path.components() {
                match part {
                    Component::RootDir => {}
                    Component::Normal(name) => {
                        let metadata = directory.metadata(name)?;
                        directory = directory.open_directory(name, metadata)?;
                    }
                    _ => return Err(io::ErrorKind::InvalidInput.into()),
                }
            }
            Ok(directory)
        }

        // stat device/inode field types differ across Darwin/Linux targets.
        #[allow(clippy::unnecessary_cast)]
        pub(crate) fn metadata(&self, name: &OsStr) -> io::Result<Metadata> {
            let name = component(name)?;
            let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: parent is live; name is NUL-terminated; fstatat writes
            // the entire stat on success. AT_SYMLINK_NOFOLLOW never enters a link.
            let result = unsafe {
                libc::fstatat(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    value.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the successful fstatat initialized value above.
            let value = unsafe { value.assume_init() };
            Ok(Metadata {
                dev: value.st_dev as u64,
                ino: value.st_ino as u64,
                mode: value.st_mode,
                len: u64::try_from(value.st_size).unwrap_or(u64::MAX),
            })
        }

        pub(crate) fn open_directory(&self, name: &OsStr, located: Metadata) -> io::Result<Self> {
            if located.is_symlink() {
                return Err(io::Error::from_raw_os_error(libc::ELOOP));
            }
            if !located.is_dir() {
                return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
            }
            let file = self.open_component(name, SEARCH_ONLY | libc::O_DIRECTORY)?;
            self.confirm_open(name, located, &file)?;
            Ok(Self(file))
        }

        pub(crate) fn open_file(&self, name: &OsStr, located: Metadata) -> io::Result<File> {
            let file = self.open_component(name, libc::O_NONBLOCK)?;
            let opened = Metadata::from_file(&file.metadata()?);
            if !opened.is_file() {
                return Err(io::Error::from_raw_os_error(libc::ENXIO));
            }
            self.confirm_open(name, located, &file)?;
            Ok(file)
        }

        fn open_component(&self, name: &OsStr, flags: libc::c_int) -> io::Result<File> {
            let c_name = component(name)?;
            // SAFETY: parent is live and name is one NUL-terminated component.
            // O_RDONLY plus no-follow flags cannot write or follow a symlink.
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | flags,
                )
            };
            if fd < 0 {
                let error = io::Error::last_os_error();
                // Darwin and Linux can return ENOTDIR rather than ELOOP for
                // O_DIRECTORY on a link. Classify only through the same parent.
                if error.raw_os_error() == Some(libc::ENOTDIR)
                    && self
                        .metadata(name)
                        .is_ok_and(|metadata| metadata.is_symlink())
                {
                    return Err(io::Error::from_raw_os_error(libc::ELOOP));
                }
                return Err(error);
            }
            // SAFETY: openat returned a fresh descriptor; File owns it once.
            Ok(unsafe { File::from_raw_fd(fd) })
        }

        fn confirm_open(&self, name: &OsStr, located: Metadata, file: &File) -> io::Result<()> {
            let opened = Metadata::from_file(&file.metadata()?);
            if !located.same_object(opened) {
                return Err(io::ErrorKind::Other.into());
            }
            self.confirm(name, opened)
        }

        pub(crate) fn confirm(&self, name: &OsStr, located: Metadata) -> io::Result<()> {
            let now = self.metadata(name)?;
            if now.is_symlink() {
                return Err(io::Error::from_raw_os_error(libc::ELOOP));
            }
            if !located.same_object(now) {
                return Err(io::ErrorKind::Other.into());
            }
            Ok(())
        }

        /// A fresh open of `.` uses this handle, with a separate directory
        /// offset. The iterator stops at `limit`, excluding dot entries.
        pub(crate) fn names(&self, limit: usize) -> io::Result<Vec<std::ffi::OsString>> {
            let name = c".";
            // SAFETY: self is live; `.` is a fixed component of this handle.
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: this new descriptor has exactly one owner.
            let file = unsafe { File::from_raw_fd(fd) };
            let fd = file.into_raw_fd();
            // SAFETY: fd is an owned directory descriptor. fdopendir takes
            // ownership only on success; the failure branch closes it below.
            let stream = unsafe { libc::fdopendir(fd) };
            let Some(stream) = NonNull::new(stream) else {
                let error = io::Error::last_os_error();
                // SAFETY: fdopendir failed, so ownership is still ours.
                drop(unsafe { File::from_raw_fd(fd) });
                return Err(error);
            };
            let stream = DirectoryStream(stream);
            let mut names = Vec::new();
            while names.len() < limit {
                // SAFETY: this thread owns stream; errno is thread-local.
                unsafe {
                    *errno() = 0;
                }
                // SAFETY: stream is live and owned only here. The entry is
                // copied before the next readdir, which could invalidate it.
                let entry = unsafe { libc::readdir(stream.0.as_ptr()) };
                if entry.is_null() {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(0) {
                        return Err(error);
                    }
                    break;
                }
                // SAFETY: a successful readdir returns a NUL-terminated d_name.
                // Darwin records can be shorter than the declared d_name
                // array. Take only its raw address, never a reference to it.
                let bytes = unsafe { CStr::from_ptr(std::ptr::addr_of!((*entry).d_name).cast()) }
                    .to_bytes();
                if bytes != b"." && bytes != b".." {
                    names.push(OsStr::from_bytes(bytes).to_owned());
                }
            }
            Ok(names)
        }
    }

    fn component(name: &OsStr) -> io::Result<CString> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        CString::new(bytes).map_err(|_| io::ErrorKind::InvalidInput.into())
    }

    #[cfg(target_os = "macos")]
    unsafe fn errno() -> *mut libc::c_int {
        // SAFETY: libc exposes this thread's errno pointer.
        unsafe { libc::__error() }
    }
    #[cfg(target_os = "linux")]
    unsafe fn errno() -> *mut libc::c_int {
        // SAFETY: libc exposes this thread's errno pointer.
        unsafe { libc::__errno_location() }
    }

    struct DirectoryStream(NonNull<libc::DIR>);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            // SAFETY: this wrapper uniquely owns the stream and descriptor.
            unsafe {
                libc::closedir(self.0.as_ptr());
            }
        }
    }
}

// These handle implementations cover macOS and Linux. Other builds fail closed, never falling
// back to path-based IO that could enter a replaced directory.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod native {
    use std::{
        ffi::{OsStr, OsString},
        fs::File,
        io,
        path::Path,
    };
    #[derive(Clone, Copy)]
    pub(crate) struct Metadata;
    impl Metadata {
        pub(crate) fn is_dir(self) -> bool {
            false
        }
        pub(crate) fn is_file(self) -> bool {
            false
        }
        pub(crate) fn is_symlink(self) -> bool {
            false
        }
        pub(crate) fn len(self) -> u64 {
            0
        }
        pub(crate) fn same_object(self, _: Self) -> bool {
            false
        }
    }
    pub(crate) struct DirHandle;
    impl DirHandle {
        pub(crate) fn clone_handle(&self) -> io::Result<Self> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub(crate) fn open_root(_: &Path) -> io::Result<Self> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub(crate) fn metadata(&self, _: &OsStr) -> io::Result<Metadata> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub(crate) fn open_directory(&self, _: &OsStr, _: Metadata) -> io::Result<Self> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub(crate) fn open_file(&self, _: &OsStr, _: Metadata) -> io::Result<File> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub(crate) fn confirm(&self, _: &OsStr, _: Metadata) -> io::Result<()> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub(crate) fn names(&self, _: usize) -> io::Result<Vec<OsString>> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

pub(super) use native::{DirHandle, Metadata};
