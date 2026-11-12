//! API-14 prefix of onnxruntime_c_api.h. Unused entries preserve their slots.
use std::ffi::{c_char, c_void};
pub(crate) type O = c_void;
pub(crate) type S = *mut O;
#[cfg(windows)]
pub(crate) type OrtChar = u16;
#[cfg(not(windows))]
pub(crate) type OrtChar = c_char;
#[repr(C)]
pub(crate) struct OrtApi {
    pub create_status: *const c_void,  // 0: CreateStatus
    pub get_error_code: *const c_void, // 1: GetErrorCode
    pub get_error_message: unsafe extern "system" fn(*const O) -> *const c_char, // 2: GetErrorMessage
    pub create_env: unsafe extern "system" fn(i32, *const c_char, *mut *mut O) -> S, // 3: CreateEnv
    pub create_env_with_custom_logger: *const c_void, // 4: CreateEnvWithCustomLogger
    pub enable_telemetry_events: *const c_void,       // 5: EnableTelemetryEvents
    pub disable_telemetry_events: *const c_void,      // 6: DisableTelemetryEvents
    pub create_session:
        unsafe extern "system" fn(*const O, *const OrtChar, *const O, *mut *mut O) -> S, // 7: CreateSession
    pub create_session_from_array: *const c_void, // 8: CreateSessionFromArray
    pub run: unsafe extern "system" fn(
        *mut O,
        *const O,
        *const *const c_char,
        *const *const O,
        usize,
        *const *const c_char,
        usize,
        *mut *mut O,
    ) -> S, // 9: Run
    pub create_session_options: unsafe extern "system" fn(*mut *mut O) -> S, // 10: CreateSessionOptions
    pub set_optimized_model_file_path: *const c_void, // 11: SetOptimizedModelFilePath
    pub clone_session_options: *const c_void,         // 12: CloneSessionOptions
    pub set_session_execution_mode: *const c_void,    // 13: SetSessionExecutionMode
    pub enable_profiling: *const c_void,              // 14: EnableProfiling
    pub disable_profiling: *const c_void,             // 15: DisableProfiling
    pub enable_mem_pattern: *const c_void,            // 16: EnableMemPattern
    pub disable_mem_pattern: *const c_void,           // 17: DisableMemPattern
    pub enable_cpu_mem_arena: *const c_void,          // 18: EnableCpuMemArena
    pub disable_cpu_mem_arena: *const c_void,         // 19: DisableCpuMemArena
    pub set_session_log_id: *const c_void,            // 20: SetSessionLogId
    pub set_session_log_verbosity_level: *const c_void, // 21: SetSessionLogVerbosityLevel
    pub set_session_log_severity_level: *const c_void, // 22: SetSessionLogSeverityLevel
    pub set_session_graph_optimization_level: *const c_void, // 23: SetSessionGraphOptimizationLevel
    pub set_intra_op_num_threads: unsafe extern "system" fn(*mut O, i32) -> S, // 24: SetIntraOpNumThreads
    pub set_inter_op_num_threads: *const c_void, // 25: SetInterOpNumThreads
    pub create_custom_op_domain: *const c_void,  // 26: CreateCustomOpDomain
    pub custom_op_domain_add: *const c_void,     // 27: CustomOpDomain_Add
    pub add_custom_op_domain: *const c_void,     // 28: AddCustomOpDomain
    pub register_custom_ops_library: *const c_void, // 29: RegisterCustomOpsLibrary
    pub session_get_input_count: unsafe extern "system" fn(*const O, *mut usize) -> S, // 30: SessionGetInputCount
    pub session_get_output_count: unsafe extern "system" fn(*const O, *mut usize) -> S, // 31: SessionGetOutputCount
    pub session_get_overridable_initializer_count: *const c_void, // 32: SessionGetOverridableInitializerCount
    pub session_get_input_type_info: unsafe extern "system" fn(*const O, usize, *mut *mut O) -> S, // 33: SessionGetInputTypeInfo
    pub session_get_output_type_info: unsafe extern "system" fn(*const O, usize, *mut *mut O) -> S, // 34: SessionGetOutputTypeInfo
    pub session_get_overridable_initializer_type_info: *const c_void, // 35: SessionGetOverridableInitializerTypeInfo
    pub session_get_input_name:
        unsafe extern "system" fn(*const O, usize, *mut O, *mut *mut c_char) -> S, // 36: SessionGetInputName
    pub session_get_output_name:
        unsafe extern "system" fn(*const O, usize, *mut O, *mut *mut c_char) -> S, // 37: SessionGetOutputName
    pub session_get_overridable_initializer_name: *const c_void, // 38: SessionGetOverridableInitializerName
    pub create_run_options: *const c_void,                       // 39: CreateRunOptions
    pub run_options_set_run_log_verbosity_level: *const c_void, // 40: RunOptionsSetRunLogVerbosityLevel
    pub run_options_set_run_log_severity_level: *const c_void, // 41: RunOptionsSetRunLogSeverityLevel
    pub run_options_set_run_tag: *const c_void,                // 42: RunOptionsSetRunTag
    pub run_options_get_run_log_verbosity_level: *const c_void, // 43: RunOptionsGetRunLogVerbosityLevel
    pub run_options_get_run_log_severity_level: *const c_void, // 44: RunOptionsGetRunLogSeverityLevel
    pub run_options_get_run_tag: *const c_void,                // 45: RunOptionsGetRunTag
    pub run_options_set_terminate: *const c_void,              // 46: RunOptionsSetTerminate
    pub run_options_unset_terminate: *const c_void,            // 47: RunOptionsUnsetTerminate
    pub create_tensor_as_ort_value: *const c_void,             // 48: CreateTensorAsOrtValue
    pub create_tensor_with_data_as_ort_value: unsafe extern "system" fn(
        *const O,
        *mut c_void,
        usize,
        *const i64,
        usize,
        i32,
        *mut *mut O,
    ) -> S, // 49: CreateTensorWithDataAsOrtValue
    pub is_tensor: *const c_void,                              // 50: IsTensor
    pub get_tensor_mutable_data: unsafe extern "system" fn(*mut O, *mut *mut c_void) -> S, // 51: GetTensorMutableData
    pub fill_string_tensor: *const c_void, // 52: FillStringTensor
    pub get_string_tensor_data_length: *const c_void, // 53: GetStringTensorDataLength
    pub get_string_tensor_content: *const c_void, // 54: GetStringTensorContent
    pub cast_type_info_to_tensor_info: unsafe extern "system" fn(*const O, *mut *const O) -> S, // 55: CastTypeInfoToTensorInfo
    pub get_onnx_type_from_type_info: *const c_void, // 56: GetOnnxTypeFromTypeInfo
    pub create_tensor_type_and_shape_info: *const c_void, // 57: CreateTensorTypeAndShapeInfo
    pub set_tensor_element_type: *const c_void,      // 58: SetTensorElementType
    pub set_dimensions: *const c_void,               // 59: SetDimensions
    pub get_tensor_element_type: unsafe extern "system" fn(*const O, *mut i32) -> S, // 60: GetTensorElementType
    pub get_dimensions_count: unsafe extern "system" fn(*const O, *mut usize) -> S, // 61: GetDimensionsCount
    pub get_dimensions: unsafe extern "system" fn(*const O, *mut i64, usize) -> S, // 62: GetDimensions
    pub get_symbolic_dimensions: *const c_void, // 63: GetSymbolicDimensions
    pub get_tensor_shape_element_count: unsafe extern "system" fn(*const O, *mut usize) -> S, // 64: GetTensorShapeElementCount
    pub get_tensor_type_and_shape: unsafe extern "system" fn(*const O, *mut *mut O) -> S, // 65: GetTensorTypeAndShape
    pub get_type_info: *const c_void,      // 66: GetTypeInfo
    pub get_value_type: *const c_void,     // 67: GetValueType
    pub create_memory_info: *const c_void, // 68: CreateMemoryInfo
    pub create_cpu_memory_info: unsafe extern "system" fn(i32, i32, *mut *mut O) -> S, // 69: CreateCpuMemoryInfo
    pub compare_memory_info: *const c_void, // 70: CompareMemoryInfo
    pub memory_info_get_name: *const c_void, // 71: MemoryInfoGetName
    pub memory_info_get_id: *const c_void,  // 72: MemoryInfoGetId
    pub memory_info_get_mem_type: *const c_void, // 73: MemoryInfoGetMemType
    pub memory_info_get_type: *const c_void, // 74: MemoryInfoGetType
    pub allocator_alloc: *const c_void,     // 75: AllocatorAlloc
    pub allocator_free: unsafe extern "system" fn(*mut O, *mut c_void) -> S, // 76: AllocatorFree
    pub allocator_get_info: *const c_void,  // 77: AllocatorGetInfo
    pub get_allocator_with_default_options: unsafe extern "system" fn(*mut *mut O) -> S, // 78: GetAllocatorWithDefaultOptions
    pub add_free_dimension_override: *const c_void, // 79: AddFreeDimensionOverride
    pub get_value: *const c_void,                   // 80: GetValue
    pub get_value_count: *const c_void,             // 81: GetValueCount
    pub create_value: *const c_void,                // 82: CreateValue
    pub create_opaque_value: *const c_void,         // 83: CreateOpaqueValue
    pub get_opaque_value: *const c_void,            // 84: GetOpaqueValue
    pub kernel_info_get_attribute_float: *const c_void, // 85: KernelInfoGetAttribute_float
    pub kernel_info_get_attribute_int64: *const c_void, // 86: KernelInfoGetAttribute_int64
    pub kernel_info_get_attribute_string: *const c_void, // 87: KernelInfoGetAttribute_string
    pub kernel_context_get_input_count: *const c_void, // 88: KernelContext_GetInputCount
    pub kernel_context_get_output_count: *const c_void, // 89: KernelContext_GetOutputCount
    pub kernel_context_get_input: *const c_void,    // 90: KernelContext_GetInput
    pub kernel_context_get_output: *const c_void,   // 91: KernelContext_GetOutput
    pub release_env: unsafe extern "system" fn(*mut O), // 92: ReleaseEnv
    pub release_status: unsafe extern "system" fn(*mut O), // 93: ReleaseStatus
    pub release_memory_info: unsafe extern "system" fn(*mut O), // 94: ReleaseMemoryInfo
    pub release_session: unsafe extern "system" fn(*mut O), // 95: ReleaseSession
    pub release_value: unsafe extern "system" fn(*mut O), // 96: ReleaseValue
    pub release_run_options: unsafe extern "system" fn(*mut O), // 97: ReleaseRunOptions
    pub release_type_info: unsafe extern "system" fn(*mut O), // 98: ReleaseTypeInfo
    pub release_tensor_type_and_shape_info: unsafe extern "system" fn(*mut O), // 99: ReleaseTensorTypeAndShapeInfo
    pub release_session_options: unsafe extern "system" fn(*mut O), // 100: ReleaseSessionOptions
}
#[repr(C)]
pub(crate) struct OrtApiBase {
    pub get_api: unsafe extern "system" fn(u32) -> *const OrtApi,
    pub get_version_string: unsafe extern "system" fn() -> *const c_char,
}
