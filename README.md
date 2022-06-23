# Rust Audio

## Todos:
- Create a Slider component with consistent styles
- Add an audio loading / initialization state style / animation
- Make initial load much faster - save a raw audio Vec for direct access at initialization

- Memoize decoded audio from previous files? To prevent stutter on change?
- Enable draggging the current buffer selection window?
- Share a single audio context that is initialized (?) at init time?
- Fix HtmlSelectElement UI interaction:
    - Require a form submit
    - Make default audio the currently selected file in the select element
    - Disable when audio has not yet been enabled

- add grain length sliders

- Web
   - Create a dropdown for switching out audio buffers

    - Build Interactive UI
        - Refactor visual representation of current audio buffer to use an svg <path /> element?
        - Create sliders/knobs for adjust grain length
        - Make nicer styles
    - Fixes:
        - Clean up logic around buffer selection ranges -- ensure no empty ranges


- Refactor for shared code in `/common`
    - Generalize multi-channel mixing for any number of runtime channels
    - Extract logic for mixing down multi-channel audio into lower-channel audio

- CLI
    - Re-sample any audio files that don't match the current sample rate.

- Add reverb (and other effects)


--------------------------

General Fixes:

 - Correct sample rate for mp3 audio for both CLI and web.