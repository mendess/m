package xyz.mendess.jukebox.ui.main

import android.os.Bundle
import android.util.Log
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import androidx.fragment.app.Fragment
import androidx.lifecycle.ViewModelProviders
import xyz.mendess.jukebox.JukeboxLib
import xyz.mendess.jukebox.R

class Home : Fragment() {

    private val TAG: String = "Home"

    companion object {
        fun newInstance() = Home()
    }

    private lateinit var viewModel: HomeViewModel

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View? {
        val root = inflater.inflate(R.layout.home_fragment, container, false)
        val mediaButtons = arrayOf(
            root.findViewById<Button>(R.id.next_button)
                .apply {
                    setOnClickListener {
                        Log.i(TAG, "next button")
                        JukeboxLib.sendCommand("next-file")
                    }
                },
            root.findViewById<Button>(R.id.prev_button)
                .apply {
                    setOnClickListener {
                        Log.i(TAG, "prev button")
                        JukeboxLib.sendCommand("prev-file")
                    }
                },
            root.findViewById<Button>(R.id.pause_button)
                .apply {
                    setOnClickListener {
                        Log.i(TAG, "pause button")
                        JukeboxLib.sendCommand("pause")
                    }
                },
            root.findViewById<Button>(R.id.volume_up_button)
                .apply {
                    setOnClickListener {
                        Log.i(TAG, "vu button")
                        JukeboxLib.sendCommand("vu")
                    }
                },
            root.findViewById<Button>(R.id.volume_down_button)
                .apply {
                    setOnClickListener {
                        Log.i(TAG, "vd button")
                        JukeboxLib.sendCommand("vd")
                    }
                }
        )
        val roomName = root.findViewById<EditText>(R.id.room_name)
        root.findViewById<Button>(R.id.room_name_submit).setOnClickListener { join_button ->
            val name = roomName.text.toString()
            Log.i(TAG, "Submitting room name '$name'")
            JukeboxLib.setRoomName(name)
            mediaButtons.forEach { it.isEnabled = true }
            join_button.isEnabled = false
        }
        return root
    }

    override fun onActivityCreated(savedInstanceState: Bundle?) {
        super.onActivityCreated(savedInstanceState)
        viewModel = ViewModelProviders.of(this).get(HomeViewModel::class.java)
        // TODO: Use the ViewModel
    }

}