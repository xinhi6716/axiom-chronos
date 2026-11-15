use super::{
    api::{OrtApi, O},
    Runtime,
};
use crate::base::{Failure, Res};
use std::{
    ffi::{c_void, CStr, CString},
    path::Path,
    ptr,
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Tensor {
    I64 { shape: Vec<i64>, data: Vec<i64> },
    F32 { shape: Vec<i64>, data: Vec<f32> },
    Bool { shape: Vec<i64>, data: Vec<bool> },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElemType {
    I64,
    F32,
    Bool,
    Other(i32),
}
impl From<i32> for ElemType {
    fn from(v: i32) -> Self {
        match v {
            1 => Self::F32,
            7 => Self::I64,
            9 => Self::Bool,
            x => Self::Other(x),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoInfo {
    pub name: String,
    pub elem: ElemType,
    pub dims: Vec<i64>,
}

fn error(msg: impl Into<String>) -> Failure {
    Failure::new("laya_ort", msg)
}
unsafe fn status(api: &OrtApi, s: *mut O) -> Res<()> {
    if s.is_null() {
        return Ok(());
    }
    let msg = (api.get_error_message)(s);
    let text = if msg.is_null() {
        "unknown ONNX Runtime error".to_owned()
    } else {
        CStr::from_ptr(msg).to_string_lossy().into_owned()
    };
    (api.release_status)(s);
    Err(error(text))
}
struct Owned {
    ptr: *mut O,
    release: unsafe extern "system" fn(*mut O),
}
impl Owned {
    fn new(_api: &OrtApi, release: unsafe extern "system" fn(*mut O)) -> Self {
        Self {
            ptr: ptr::null_mut(),
            release,
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.release)(self.ptr) }
        }
    }
}

pub struct Session {
    rt: Arc<Runtime>,
    session: *mut O,
    env: *mut O,
    inputs: Vec<IoInfo>,
    outputs: Vec<IoInfo>,
}
// ORT permits concurrent Run calls on a session. Its handles and immutable metadata stay alive
// through the Arc and are released only after all references to the Session have gone away.
unsafe impl Send for Session {}
unsafe impl Sync for Session {}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            (self.rt.api.release_session)(self.session);
            (self.rt.api.release_env)(self.env);
        }
    }
}

impl Session {
    pub fn new(rt: &Arc<Runtime>, model: &Path, intra_threads: usize) -> Res<Self> {
        let api = rt.api;
        let threads =
            i32::try_from(intra_threads).map_err(|_| error("intra_threads exceeds i32"))?;
        #[cfg(windows)]
        let path: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            let mut p: Vec<_> = model.as_os_str().encode_wide().collect();
            if p.contains(&0) {
                return Err(error("model path contains NUL"));
            }
            p.push(0);
            p
        };
        #[cfg(not(windows))]
        let path = {
            use std::os::unix::ffi::OsStrExt;
            CString::new(model.as_os_str().as_bytes())
                .map_err(|_| error("model path contains NUL"))?
        };
        #[cfg(windows)]
        let path_ptr = path.as_ptr();
        #[cfg(not(windows))]
        let path_ptr = path.as_ptr();
        unsafe {
            let mut env = Owned::new(api, api.release_env);
            status(
                api,
                (api.create_env)(2, b"axiom-laya\0".as_ptr().cast(), &mut env.ptr),
            )?;
            let mut options = Owned::new(api, api.release_session_options);
            status(api, (api.create_session_options)(&mut options.ptr))?;
            status(api, (api.set_intra_op_num_threads)(options.ptr, threads))?;
            let mut session = Owned::new(api, api.release_session);
            status(
                api,
                (api.create_session)(env.ptr, path_ptr, options.ptr, &mut session.ptr),
            )?;
            let inputs = io_info(api, session.ptr, true)?;
            let outputs = io_info(api, session.ptr, false)?;
            let result = Self {
                rt: rt.clone(),
                session: session.ptr,
                env: env.ptr,
                inputs,
                outputs,
            };
            session.ptr = ptr::null_mut();
            env.ptr = ptr::null_mut();
            Ok(result)
        }
    }
    pub fn inputs(&self) -> &[IoInfo] {
        &self.inputs
    }
    pub fn outputs(&self) -> &[IoInfo] {
        &self.outputs
    }
    pub fn run(&self, inputs: &[(&str, Tensor)], outputs: &[&str]) -> Res<Vec<Tensor>> {
        let api = self.rt.api;
        let input_names: Vec<CString> = inputs
            .iter()
            .map(|(n, _)| CString::new(*n).map_err(|_| error("input name contains NUL")))
            .collect::<Res<_>>()?;
        let output_names: Vec<CString> = outputs
            .iter()
            .map(|n| CString::new(*n).map_err(|_| error("output name contains NUL")))
            .collect::<Res<_>>()?;
        let in_names: Vec<_> = input_names.iter().map(|n| n.as_ptr()).collect();
        let out_names: Vec<_> = output_names.iter().map(|n| n.as_ptr()).collect();
        // ORT borrows these buffers for the duration of Run. BOOL inputs use an
        // explicit byte-per-element representation for the C API.
        let mut backing: Vec<Backing> = inputs
            .iter()
            .map(|(_, t)| Backing::from_tensor(t))
            .collect::<Res<_>>()?;
        unsafe {
            let mut memory = Owned::new(api, api.release_memory_info);
            status(api, (api.create_cpu_memory_info)(1, 0, &mut memory.ptr))?;
            let mut values = Vec::with_capacity(backing.len());
            for b in &mut backing {
                let mut v = Owned::new(api, api.release_value);
                let (data, bytes, shape, elem) = b.parts();
                status(
                    api,
                    (api.create_tensor_with_data_as_ort_value)(
                        memory.ptr,
                        data,
                        bytes,
                        shape.as_ptr(),
                        shape.len(),
                        elem,
                        &mut v.ptr,
                    ),
                )?;
                values.push(v);
            }
            let in_values: Vec<*const O> = values.iter().map(|v| v.ptr as *const O).collect();
            let mut raw_outputs = vec![ptr::null_mut(); outputs.len()];
            let result = status(
                api,
                (api.run)(
                    self.session,
                    ptr::null(),
                    in_names.as_ptr(),
                    in_values.as_ptr(),
                    in_values.len(),
                    out_names.as_ptr(),
                    out_names.len(),
                    raw_outputs.as_mut_ptr(),
                ),
            );
            let owned_outputs: Vec<_> = raw_outputs
                .into_iter()
                .map(|p| Owned {
                    ptr: p,
                    release: api.release_value,
                })
                .collect();
            result?;
            owned_outputs
                .iter()
                .map(|v| read_tensor(api, v.ptr))
                .collect()
        }
    }
}

unsafe fn io_info(api: &OrtApi, session: *mut O, input: bool) -> Res<Vec<IoInfo>> {
    let mut count = 0;
    status(
        api,
        if input {
            (api.session_get_input_count)(session, &mut count)
        } else {
            (api.session_get_output_count)(session, &mut count)
        },
    )?;
    let mut allocator = ptr::null_mut();
    status(
        api,
        (api.get_allocator_with_default_options)(&mut allocator),
    )?;
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let mut name = ptr::null_mut();
        status(
            api,
            if input {
                (api.session_get_input_name)(session, i, allocator, &mut name)
            } else {
                (api.session_get_output_name)(session, i, allocator, &mut name)
            },
        )?;
        if name.is_null() {
            return Err(error("ORT returned a null name"));
        }
        let text = CStr::from_ptr(name).to_string_lossy().into_owned();
        status(api, (api.allocator_free)(allocator, name.cast()))?;
        let mut info = Owned::new(api, api.release_type_info);
        status(
            api,
            if input {
                (api.session_get_input_type_info)(session, i, &mut info.ptr)
            } else {
                (api.session_get_output_type_info)(session, i, &mut info.ptr)
            },
        )?;
        let mut tensor_info = ptr::null();
        status(
            api,
            (api.cast_type_info_to_tensor_info)(info.ptr, &mut tensor_info),
        )?;
        if tensor_info.is_null() {
            return Err(error(format!("{text} is not a tensor")));
        }
        let (elem, dims) = shape_info(api, tensor_info)?;
        result.push(IoInfo {
            name: text,
            elem,
            dims,
        });
    }
    Ok(result)
}
unsafe fn shape_info(api: &OrtApi, info: *const O) -> Res<(ElemType, Vec<i64>)> {
    let mut elem = 0;
    let mut n = 0;
    status(api, (api.get_tensor_element_type)(info, &mut elem))?;
    status(api, (api.get_dimensions_count)(info, &mut n))?;
    let mut dims = vec![0; n];
    status(api, (api.get_dimensions)(info, dims.as_mut_ptr(), n))?;
    Ok((elem.into(), dims))
}
fn count(shape: &[i64]) -> Res<usize> {
    shape.iter().try_fold(1usize, |n, &d| {
        usize::try_from(d)
            .ok()
            .and_then(|d| n.checked_mul(d))
            .ok_or_else(|| error("invalid or overflowing tensor shape"))
    })
}
enum Backing {
    I64(Vec<i64>, Vec<i64>),
    F32(Vec<i64>, Vec<f32>),
    Bool(Vec<i64>, Vec<u8>),
}
impl Backing {
    fn from_tensor(t: &Tensor) -> Res<Self> {
        let (shape, len) = match t {
            Tensor::I64 { shape, data } => (shape, data.len()),
            Tensor::F32 { shape, data } => (shape, data.len()),
            Tensor::Bool { shape, data } => (shape, data.len()),
        };
        if count(shape)? != len {
            return Err(error("tensor data length does not match shape"));
        }
        Ok(match t {
            Tensor::I64 { shape, data } => Self::I64(shape.clone(), data.clone()),
            Tensor::F32 { shape, data } => Self::F32(shape.clone(), data.clone()),
            Tensor::Bool { shape, data } => {
                Self::Bool(shape.clone(), data.iter().map(|v| u8::from(*v)).collect())
            }
        })
    }
    fn parts(&mut self) -> (*mut c_void, usize, &[i64], i32) {
        match self {
            Self::I64(s, d) => (d.as_mut_ptr().cast(), d.len() * 8, s, 7),
            Self::F32(s, d) => (d.as_mut_ptr().cast(), d.len() * 4, s, 1),
            Self::Bool(s, d) => (d.as_mut_ptr().cast(), d.len(), s, 9),
        }
    }
}
unsafe fn read_tensor(api: &OrtApi, value: *mut O) -> Res<Tensor> {
    if value.is_null() {
        return Err(error("ORT returned a null output"));
    }
    let mut info = Owned::new(api, api.release_tensor_type_and_shape_info);
    status(api, (api.get_tensor_type_and_shape)(value, &mut info.ptr))?;
    let (elem, shape) = shape_info(api, info.ptr)?;
    let mut len = 0;
    status(
        api,
        (api.get_tensor_shape_element_count)(info.ptr, &mut len),
    )?;
    if count(&shape)? != len {
        return Err(error("invalid output shape"));
    }
    let mut data = ptr::null_mut();
    status(api, (api.get_tensor_mutable_data)(value, &mut data))?;
    if len > 0 && data.is_null() {
        return Err(error("ORT returned null tensor data"));
    }
    if len > isize::MAX as usize / 8 {
        return Err(error("output tensor is too large"));
    }
    // Avoid forming a slice from a null pointer for an empty tensor.
    Ok(match elem {
        ElemType::I64 => Tensor::I64 {
            shape,
            data: if len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data.cast::<i64>(), len).to_vec()
            },
        },
        ElemType::F32 => Tensor::F32 {
            shape,
            data: if len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data.cast::<f32>(), len).to_vec()
            },
        },
        ElemType::Bool => Tensor::Bool {
            shape,
            data: if len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data.cast::<u8>(), len)
                    .iter()
                    .map(|&v| v != 0)
                    .collect()
            },
        },
        ElemType::Other(code) => {
            return Err(error(format!(
                "unsupported output tensor element type {code}"
            )))
        }
    })
}
