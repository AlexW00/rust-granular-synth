use crate::audio::buffer_handle::BufferHandle;
use crate::audio::buffer_selection_handle::BufferSelectionHandle;
use crate::audio::current_status_handle::CurrentStatusHandle;
use crate::audio::gain_handle::GainHandle;
use crate::audio::stream_handle::StreamHandle;
use crate::components::buffer_sample_bars::get_buffer_maxes;
use crate::state::app_action::AppAction;
use std::rc::Rc;
use yew::Reducible;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AppState {
    /// The currently loaded audio buffer
    pub buffer: BufferHandle,
    /// A list with a set length of max amplitudes from the original audio buffer
    /// this makes re-rendering the audio buffer visualization O(1) instead of O(n),
    /// where n is the length of buffer samples.
    ///
    /// The audio amlitudes range from 0.0 -> 100.0 and are formatted as strings to
    /// the tens decimal place.
    pub buffer_maxes: Vec<String>,
    /// A handle to the audio context stream (keeps audio playing & stops audio when dropped)
    pub stream_handle: Option<StreamHandle>,
    /// Represents what portion of the audio buffer is currently selected
    pub buffer_selection_handle: BufferSelectionHandle,
    /// Overall audio gain for output audio
    pub gain_handle: GainHandle,
    /// Current play / pause status
    pub current_status_handle: CurrentStatusHandle,
    /// Has audio been initialized yet? Audio interactions must be initiated from 
    /// a user touch / interaction. 
    pub audio_initialized: bool,
    /// If audio currently being initialized?
    pub audio_loading: bool,
}

impl Reducible for AppState {
    type Action = AppAction;

    fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
        let mut next_state = (*self).clone();
        {
            let action = action.clone();
            match action {
                AppAction::SetBuffer(buffer) => {
                    next_state.buffer_maxes = get_buffer_maxes(&buffer);
                    next_state.buffer = BufferHandle::new(buffer);
                }
                AppAction::SetStreamHandle(stream_handle) => {
                    next_state.stream_handle = stream_handle;
                }
                AppAction::SetBufferSelectionStart(start) => {
                    next_state.buffer_selection_handle.set_mouse_start(start);
                }
                AppAction::SetBufferSelectionEnd(end) => {
                    next_state.buffer_selection_handle.set_mouse_end(end);
                }
                AppAction::SetBufferSelectionMouseDown(mouse_down) => {
                    next_state.buffer_selection_handle.set_mouse_down(mouse_down);
                }
                AppAction::SetGain(gain) => {
                    next_state.gain_handle.set(gain);
                }
                AppAction::SetStatus(current_status) => {
                    next_state.current_status_handle.set(current_status);
                }
                AppAction::SetAudioInitialized(is_initialized) => {
                    next_state.audio_initialized = is_initialized;
                },
                AppAction::SetAudioLoading(loading) => {
                    next_state.audio_loading = loading;
                },
            }
        }

        Rc::new(next_state)
    }
}
